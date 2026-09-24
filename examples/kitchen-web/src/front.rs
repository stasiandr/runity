//! What the players see and press around the kitchen: the menu (host, join
//! a friend), the lobby while the doors are shut (who is in, the host's
//! start), the HUD over the round (the clock, the score, the orders, who is
//! in), the results at the end — the screens in `ui/`, filled from the
//! round as the host sends it — and the keys, turned into walking for a
//! player's own cook and [`Act`]s for what their hands do.
//!
//! Who plays which cook is the host's to say ([`seat`]); each player then
//! drives their own ([`claim`]): one cook a player.

use std::collections::HashMap;

use scrap::glam::Vec2;
use scrap::hecs::{Entity, World};
use scrap::input::Input;
use scrap::party::Party;
use scrap::screen::{Anchor, Element, Kind, Layout, Screen};
use scrap::strings::Strings;
use scrap::ui::Ui;
use scrap::widgets::Widgets;
use scrap::Actions;

use crate::components::Player;
use crate::state::{Act, Controls, Round, Seat, ACT};

/// Where the game is.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Menu,
    /// Dialled, waiting for the host's world.
    Joining,
    Kitchen,
}

/// What a player asked for on a screen.
#[derive(Debug, Clone, PartialEq)]
pub enum Wish {
    /// Host: a Steam lobby friends can join — alone without Steam.
    Host,
    /// The Steam friends list, to join a friend's kitchen from.
    Friends,
    /// The host opens the doors: the round starts for everyone.
    Start,
    /// Steam's invite dialog, for the lobby.
    Invite,
    /// This player is ready, or not after all.
    Ready(bool),
    /// The host takes everyone to another kitchen, by scene.
    Level(&'static str),
    Again,
    /// Back to the menu, out of any session.
    Leave,
    Quit,
    /// A mixer group's volume, 0 to 1: `music` or `sfx`.
    Volume(&'static str, f32),
    /// The language, by its file in `strings/`.
    Language(&'static str),
}

/// Three stars, `won` of them filled.
pub fn stars(won: usize) -> String {
    (0..3)
        .map(|i| if i < won { "★" } else { "☆" })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The kitchens the host may choose in the lobby, in its order, by scene.
pub const LEVELS: [&str; 2] = ["main", "rush"];

/// The languages the menu offers, in its order, by file.
pub const LANGUAGES: [&str; 2] = ["en", "ru"];

pub struct Front {
    pub phase: Phase,
    pub menu: Screen,
    pub hud: Screen,
    pub results: Screen,
    pub pause: Screen,
    /// Who is in, and the host's start, while the doors are shut.
    pub lobby: Screen,
    /// The host is gone: `Some(true)` closed the kitchen, `Some(false)`
    /// stopped answering and may come back.
    pub host_lost: Option<bool>,
    pub lost: Screen,
    /// Rooms are on (in the browser): a friend's joined by code, the
    /// room's link shared.
    pub rooms: bool,
    /// The room this kitchen is, by its code, when it is one.
    pub room: Option<String>,
    /// The scene the session plays.
    pub level: String,
    /// The doors were open last frame: the chef talks as they open.
    was_open: bool,
    /// The score as last seen, and the points on their way up.
    last_score: Option<i32>,
    floaters: scrap::floaters::Floaters,
    /// Escape was pressed in the kitchen: the pause card is up.
    pub paused: bool,
    /// A card for each order, laid out for how many there are.
    orders: Screen,
    cards: usize,
    /// What the menu says under its buttons.
    pub status: String,
    /// Whether each of this player's cooks is working, as last sent.
    working: HashMap<u32, bool>,
    /// The best score on this machine, and whether this round beat it.
    pub best: i64,
    pub new_best: bool,
    /// The head chef's word as a round starts, and how long the line has
    /// been up.
    speech: Screen,
    chef: scrap::dialogue::Dialogue,
    talk: Option<(scrap::dialogue::Conversation, f32)>,
}

/// Where points are won — over the window — and lost — at the board.
const WON_AT: scrap::glam::Vec3 = scrap::glam::Vec3::new(-1.5, 1.7, 3.0);
const LOST_AT: scrap::glam::Vec3 = scrap::glam::Vec3::new(0.0, 2.3, -3.8);

/// Seconds a line of the chef's stays up.
pub const LINE_SECONDS: f32 = 2.8;

impl Front {
    pub fn load(ui_dir: &std::path::Path) -> Result<Self, String> {
        let chef = ui_dir
            .with_file_name(scrap::dialogue::DIR)
            .join("chef.ron");
        let screen = |name: &str| Screen::load(ui_dir.join(format!("{name}.ron")));
        Ok(Self {
            phase: Phase::Menu,
            menu: {
                let mut menu = screen("menu")?;
                // Loud enough until the player says otherwise.
                menu.set_value("music", 0.8);
                menu.set_value("sfx", 0.8);
                menu
            },
            hud: screen("hud")?,
            results: screen("results")?,
            pause: screen("pause")?,
            lobby: screen("lobby")?,
            host_lost: None,
            lost: screen("lost")?,
            rooms: false,
            room: None,
            level: LEVELS[0].to_string(),
            was_open: false,
            last_score: None,
            floaters: scrap::floaters::Floaters::new(),
            paused: false,
            orders: Screen::from_layout(Layout::default()),
            cards: usize::MAX,
            status: String::new(),
            working: HashMap::new(),
            best: 0,
            new_best: false,
            speech: screen("speech")?,
            chef: scrap::dialogue::Dialogue::load(chef)?,
            talk: None,
        })
    }

    /// The screens saved since, reloaded; what did not parse, said.
    pub fn poll(&mut self, delta: f32) -> Vec<String> {
        [
            &mut self.menu,
            &mut self.hud,
            &mut self.results,
            &mut self.speech,
            &mut self.pause,
            &mut self.lobby,
            &mut self.lost,
        ]
        .into_iter()
        .filter_map(|s| s.poll(delta))
        .filter_map(Result::err)
        .collect()
    }

    /// Draw what the phase shows, and say what was pressed.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        world: &World,
        party: &Party,
        widgets: &mut Widgets,
        ui: &mut Ui,
        input: &Input,
        size: Vec2,
        strings: &Strings,
    ) -> Option<Wish> {
        match self.phase {
            Phase::Menu | Phase::Joining => {
                let status = if self.phase == Phase::Joining {
                    "Knocking on the kitchen door…".to_string()
                } else {
                    self.status.clone()
                };
                self.menu.set_text("status", status);
                self.menu.set_text(
                    "best",
                    if self.best > 0 {
                        format!("Best: {}", self.best)
                    } else {
                        String::new()
                    },
                );
                // A friend's kitchen is joined by its room code; a page is
                // closed, not quit.
                self.menu.set_hidden("friends", !self.rooms);
                self.menu.set_hidden("quit", cfg!(target_arch = "wasm32"));
                let done = self.menu.draw_localized(widgets, ui, input, size, strings);
                if done.clicked("friends") {
                    return Some(Wish::Friends);
                }
                if done.clicked("host") {
                    return Some(Wish::Host);
                }
                if done.clicked("quit") {
                    return Some(Wish::Quit);
                }
                for group in ["music", "sfx"] {
                    if done.changed(group) {
                        return Some(Wish::Volume(group, self.menu.value(group)));
                    }
                }
                if done.changed("language") {
                    let chosen = self.menu.chosen("language").min(LANGUAGES.len() - 1);
                    return Some(Wish::Language(LANGUAGES[chosen]));
                }
                None
            }
            Phase::Kitchen if self.host_lost.is_some() => {
                // Over everything: the host is gone.
                let quit = self.host_lost == Some(true);
                self.lost
                    .set_text("title", if quit { "@lost.quit" } else { "@lost.quiet" });
                self.lost
                    .set_text("hint", if quit { "" } else { "@lost.wait" });
                let done = self.lost.draw_localized(widgets, ui, input, size, strings);
                done.clicked("menu").then_some(Wish::Leave)
            }
            Phase::Kitchen => {
                let round = round_of(world);
                let over = round.as_ref().is_some_and(|r| r.over);
                let open = round.as_ref().is_some_and(|r| r.open);
                self.fill_hud(round.as_ref(), party);
                // The lobby has its own way out.
                self.hud.set_hidden("leave", !open);
                let mut wish = None;
                let done = self.hud.draw_localized(widgets, ui, input, size, strings);
                if done.clicked("leave") {
                    wish = Some(Wish::Leave);
                }
                if open && !self.was_open {
                    self.brief();
                }
                self.was_open = open;
                // No points rising over the results.
                self.watch_score(round.as_ref().filter(|r| r.open && !r.over));
                if !open {
                    // The lobby: who is in, and the host's start.
                    self.talk = None;
                    let host = party.is_host();
                    // Who is in, and who is ready: said on their cooks.
                    let ready = ready_players(world);
                    let roster = party.roster();
                    let set = roster.iter().filter(|(p, _)| ready.contains(&p.0)).count();
                    let is_ready = strings.resolve("@lobby.is_ready");
                    self.lobby.set_items(
                        "players",
                        roster
                            .iter()
                            .map(|(p, name)| {
                                if ready.contains(&p.0) {
                                    format!("{name} — {is_ready}")
                                } else {
                                    name.clone()
                                }
                            })
                            .collect(),
                    );
                    let mine = ready.contains(&party.me().0);
                    self.lobby.set_text(
                        "ready",
                        if mine {
                            "@lobby.unready"
                        } else {
                            "@lobby.ready"
                        },
                    );
                    let start = strings.resolve("@lobby.start");
                    self.lobby
                        .set_text("start", format!("{start} ({set}/{})", roster.len()));
                    self.lobby.set_hidden("start", !host);
                    self.lobby.set_hidden("level", !host);
                    if let Some(i) = LEVELS.iter().position(|l| *l == self.level) {
                        self.lobby.set_chosen("level", i);
                    }
                    self.lobby.set_hidden("wait", host);
                    self.lobby.set_hidden("invite", !self.rooms || !host);
                    match (&self.room, host) {
                        (Some(room), true) => self
                            .lobby
                            .set_text("hint", format!("{} {room}", strings.resolve("@lobby.room"))),
                        _ => self.lobby.set_text("hint", "@lobby.hint"),
                    }
                    let done = self.lobby.draw_localized(widgets, ui, input, size, strings);
                    if done.clicked("start") && host {
                        wish = Some(Wish::Start);
                    }
                    if done.clicked("invite") {
                        wish = Some(Wish::Invite);
                    }
                    if done.clicked("ready") {
                        wish = Some(Wish::Ready(!mine));
                    }
                    if done.changed("level") && host {
                        let chosen = self.lobby.chosen("level").min(LEVELS.len() - 1);
                        wish = Some(Wish::Level(LEVELS[chosen]));
                    }
                    if done.clicked("leave") {
                        wish = Some(Wish::Leave);
                    }
                } else if over {
                    // The round is done: no orders, and the chef is quiet.
                    self.talk = None;
                } else {
                    self.fill_orders(round.as_ref());
                    self.orders
                        .draw_localized(widgets, ui, input, size, strings);
                }
                if let Some((line, speaker)) = self.line() {
                    self.speech.set_text("line", line);
                    self.speech.set_text("speaker", speaker);
                    self.speech
                        .draw_localized(widgets, ui, input, size, strings);
                }
                if self.paused {
                    let done = self.pause.draw_localized(widgets, ui, input, size, strings);
                    if done.clicked("resume") {
                        self.paused = false;
                    }
                    if done.clicked("leave") {
                        self.paused = false;
                        wish = Some(Wish::Leave);
                    }
                    return wish;
                }
                if let Some(round) = round.filter(|r| r.over) {
                    self.results
                        .set_text("score", format!("Score {}", round.score));
                    let won = world
                        .query::<&crate::components::Kitchen>()
                        .iter()
                        .next()
                        .map_or(0, |k| k.stars_for(round.score));
                    self.results.set_text("stars", stars(won));
                    self.results.set_text(
                        "served",
                        format!(
                            "{} dish{} served",
                            round.served,
                            if round.served == 1 { "" } else { "es" }
                        ),
                    );
                    let host = party.is_host();
                    self.results.set_text(
                        "best",
                        if self.new_best {
                            "@results.new_best"
                        } else {
                            ""
                        },
                    );
                    self.results.set_text(
                        "wait",
                        if host {
                            ""
                        } else {
                            "The host starts the next round"
                        },
                    );
                    let done = self
                        .results
                        .draw_localized(widgets, ui, input, size, strings);
                    if done.clicked("again") && host {
                        wish = Some(Wish::Again);
                    }
                    if done.clicked("menu") {
                        wish = Some(Wish::Leave);
                    }
                }
                wish
            }
        }
    }

    /// Someone's "here!", over where they point.
    pub fn ping(&mut self, ping: crate::state::Ping) {
        let colour = scrap::glam::Vec4::new(0.45, 0.85, 1.0, 1.0);
        self.floaters
            .push(scrap::glam::Vec3::from_array(ping.at), "!", colour);
    }

    /// A floater for every change in the score.
    fn watch_score(&mut self, round: Option<&Round>) {
        let Some(score) = round.map(|r| r.score) else {
            self.last_score = None;
            self.floaters = scrap::floaters::Floaters::new();
            return;
        };
        if let Some(was) = self.last_score {
            let change = score - was;
            if change > 0 {
                self.floaters.push(
                    WON_AT,
                    format!("+{change}"),
                    scrap::glam::Vec4::new(1.0, 0.84, 0.3, 1.0),
                );
            } else if change < 0 {
                self.floaters.push(
                    LOST_AT,
                    format!("−{}", -change),
                    scrap::glam::Vec4::new(1.0, 0.35, 0.25, 1.0),
                );
            }
        }
        self.last_score = Some(score);
    }

    /// The floaters, placed on the screen from where they are in the
    /// kitchen as `camera` sees it: call after the camera is known.
    pub fn floaters(
        &mut self,
        camera: &scrap::render::Camera,
        size: Vec2,
        ui: &mut Ui,
        seconds: f32,
    ) {
        self.floaters.draw(camera, size, ui, seconds);
    }

    fn fill_hud(&mut self, round: Option<&Round>, party: &Party) {
        let Some(round) = round.filter(|r| r.open) else {
            self.hud.set_text("clock", "");
            self.hud.set_text("score", "");
            self.hud.set_text("players", "");
            self.hud.set_text("keys", "@hud.keys");
            return;
        };
        let seconds = round.time_left.ceil() as u32;
        self.hud
            .set_text("clock", format!("{}:{:02}", seconds / 60, seconds % 60));
        self.hud.set_text("score", format!("Score {}", round.score));
        self.hud.set_text(
            "note",
            round
                .note
                .as_ref()
                .map(|(w, _)| w.clone())
                .unwrap_or_default(),
        );
        let names: Vec<String> = party.roster().into_iter().map(|(_, n)| n).collect();
        let ping = party
            .round_trip()
            .filter(|_| !party.is_host())
            .map(|rtt| format!(" · {:.0} ms", rtt * 1000.0))
            .unwrap_or_default();
        self.hud
            .set_text("players", format!("{}{ping}", names.join(", ")));
        self.hud.set_text("keys", "@hud.keys");
    }

    /// The head chef starts talking: the dialogue from its start.
    pub fn brief(&mut self) {
        let mut flags = scrap::dialogue::State::default();
        let talk = scrap::dialogue::Conversation::begin(self.chef.clone(), &mut flags);
        self.talk = Some((talk, 0.0));
    }

    /// Time on for the chef: a line lasts [`LINE_SECONDS`], then the next.
    pub fn listen(&mut self, seconds: f32) {
        let Some((talk, shown)) = &mut self.talk else {
            return;
        };
        *shown += seconds;
        if *shown >= LINE_SECONDS {
            *shown = 0.0;
            talk.next(&mut scrap::dialogue::State::default());
            if talk.over() {
                self.talk = None;
            }
        }
    }

    /// The chef's line up now, and who says it.
    pub fn line(&self) -> Option<(String, String)> {
        let line = self.talk.as_ref()?.0.line()?;
        Some((line.text.clone(), line.speaker.clone()))
    }

    /// A card an order: what it is, and a bar of how long it will wait.
    fn fill_orders(&mut self, round: Option<&Round>) {
        let orders = round.map(|r| r.orders.clone()).unwrap_or_default();
        if orders.len() != self.cards {
            self.cards = orders.len();
            self.orders = Screen::from_layout(order_cards(orders.len()));
        }
        for (i, order) in orders.iter().enumerate() {
            let what = order.dish.key();
            self.orders.set_text(&format!("order {i}"), what);
            self.orders
                .set_value(&format!("patience {i}"), order.left / order.total);
        }
    }

    /// The keys, for the cook this player drives: walking straight onto
    /// the cook, what the hands do to the host as an act.
    pub fn drive(
        &mut self,
        world: &mut World,
        party: &mut Party,
        actions: &Actions,
        input: &Input,
    ) {
        for (cook, index) in local_cooks(world, party) {
            let x = actions.axis(input, "right");
            // Forward is away from the camera, which looks down -z.
            let z = -actions.axis(input, "forward");
            let mut controls = world.get::<&Controls>(cook).map(|c| *c).unwrap_or_default();
            controls.x = x;
            controls.z = z;
            let _ = world.insert_one(cook, controls);
            if actions.pressed(input, "grab") {
                party.publish(ACT, &Act::Grab { cook: index });
            }
            if actions.pressed(input, "throw") {
                party.publish(ACT, &Act::Throw { cook: index });
            }
            // "Here!" over the spot in front of the cook, for everyone.
            if actions.pressed(input, "ping") {
                if let Ok(t) = world.get::<&scrap::Transform>(cook) {
                    let at =
                        t.position + crate::state::facing(&t) * 1.0 + scrap::glam::Vec3::Y * 1.3;
                    party.publish(
                        crate::state::PING,
                        &crate::state::Ping { at: at.to_array() },
                    );
                }
            }
            let on = actions.held(input, "work");
            if self.working.get(&index).copied().unwrap_or(false) != on {
                self.working.insert(index, on);
                party.publish(ACT, &Act::Work { cook: index, on });
            }
        }
        let over = round_of(world).is_some_and(|r| r.over);
        if over && party.is_host() && actions.pressed(input, "restart") {
            party.publish(ACT, &Act::Restart);
        }
    }
}

/// The round as this peer has it.
/// Kenney Future, the kitchen's letters. Built into the game rather than
/// read from `assets/`: `scrap build` ships the library, not the sources.
pub const FONT: &[u8] = include_bytes!("../assets/fonts/kenney_future.ttf");

/// The kitchen's buttons and panels: warm, round, with a lip to press —
/// the colours of Kenney's UI pack.
pub fn style() -> scrap::widgets::Style {
    use scrap::glam::Vec4;
    let hex = |rgb: u32, a: f32| {
        let c = |s: u32| ((rgb >> s) & 0xff) as f32 / 255.0;
        Vec4::new(c(16), c(8), c(0), a)
    };
    scrap::widgets::Style {
        idle: hex(0xe86a17, 0.97),
        hover: hex(0xf5873a, 1.0),
        pressed: hex(0x3a2a24, 0.88),
        accent: hex(0x5aa832, 1.0),
        text: hex(0xfff6e8, 1.0),
        text_size: 18.0,
        radius: 10.0,
        bevel: 5.0,
        shadow: 6.0,
    }
}

/// The players who said they are ready, by peer: whoever sits at a cook
/// that says so.
pub fn ready_players(world: &World) -> Vec<u32> {
    world
        .query::<(&Seat, &crate::components::Ready)>()
        .iter()
        .filter(|(_, r)| r.0)
        .map(|(s, _)| s.0)
        .collect()
}

/// This player ready or not: on the cook they play, which is theirs to
/// send.
pub fn set_ready(world: &mut World, party: &Party, ready: bool) {
    for (cook, _) in local_cooks(world, party) {
        let _ = world.insert_one(cook, crate::components::Ready(ready));
    }
}

pub fn round_of(world: &World) -> Option<Round> {
    world.query::<&Round>().iter().next().cloned()
}

/// This player's cooks stand still: paused, the keys are not theirs.
pub fn halt(world: &mut World, party: &Party) {
    for (cook, _) in local_cooks(world, party) {
        if let Ok(mut c) = world.get::<&mut Controls>(cook) {
            c.x = 0.0;
            c.z = 0.0;
        }
    }
}

/// The cook this player drives, by index: the one seated for them.
pub fn local_cooks(world: &World, party: &Party) -> Vec<(Entity, u32)> {
    let me = party.me().0;
    let mut out: Vec<(Entity, u32)> = world
        .query::<(Entity, &Player, &Seat)>()
        .iter()
        .filter(|(_, _, seat)| seat.0 == me)
        .map(|(e, p, _)| (e, p.index))
        .collect();
    out.sort_by_key(|(_, i)| *i);
    out.truncate(1);
    out
}

/// The host seats the players: each player in the roster at a cook, in
/// order — alone, itself at the first. A cook nobody is seated at is out of
/// the kitchen.
pub fn seat(world: &mut World, party: &Party) {
    if !party.is_host() {
        return;
    }
    let players: Vec<u32> = if party.is_alone() {
        vec![party.me().0]
    } else {
        party.roster().into_iter().map(|(p, _)| p.0).collect()
    };
    let mut cooks: Vec<(Entity, u32, Option<Seat>)> = world
        .query::<(Entity, &Player, Option<&Seat>)>()
        .iter()
        .map(|(e, p, s)| (e, p.index, s.copied()))
        .collect();
    cooks.sort_by_key(|(_, i, _)| *i);
    for (i, (cook, _, now)) in cooks.into_iter().enumerate() {
        let wanted = players.get(i).map(|p| Seat(*p));
        if wanted != now {
            match wanted {
                Some(seat) => {
                    let _ = world.insert_one(cook, seat);
                }
                None => {
                    let _ = world.remove_one::<Seat>(cook);
                    drop_held(world, cook);
                }
            }
        }
    }
}

/// What a cook nobody plays any more was holding: down on the floor where
/// they stood, for someone else to pick up.
fn drop_held(world: &mut World, cook: Entity) {
    let Some(item) = world
        .get::<&crate::state::Hands>(cook)
        .ok()
        .and_then(|h| h.0)
    else {
        return;
    };
    let at = world
        .get::<&scrap::Transform>(cook)
        .map(|t| t.position)
        .unwrap_or_default();
    let _ = world.insert_one(cook, crate::state::Hands(None));
    let _ = world.remove_one::<crate::state::At>(item);
    if let Ok(mut t) = world.get::<&mut scrap::Transform>(item) {
        t.position = scrap::glam::Vec3::new(at.x, crate::state::FOOD_RADIUS + 0.05, at.z);
    }
}

/// Each player takes the cook seated for them; the host takes back one
/// nobody plays any more.
pub fn claim(world: &mut World, party: &Party) {
    if !party.welcomed() {
        return;
    }
    let me = party.me().0;
    let wanted: Vec<Entity> = world
        .query::<(Entity, &Player, Option<&Seat>, Option<&scrap::net::Owned>)>()
        .iter()
        .filter(|(_, _, seat, owned)| {
            let mine = seat.is_some_and(|s| s.0 == me) || (seat.is_none() && party.is_host());
            mine && owned.is_none()
        })
        .map(|(e, ..)| e)
        .collect();
    for cook in wanted {
        party.claim(world, cook);
    }
}

/// The order cards, top left: a panel, the soup's name, and the bar.
fn order_cards(count: usize) -> Layout {
    let mut elements = Vec::new();
    for i in 0..count {
        let top = 16.0 + i as f32 * 66.0;
        let element =
            |id: String, at: (f32, f32), size: (f32, f32), kind: Kind, text_size: f32| Element {
                id,
                anchor: Anchor::TopLeft,
                at,
                size,
                kind,
                text_size,
                align: Default::default(),
            };
        elements.push(element(
            format!("card {i}"),
            (16.0, top),
            (240.0, 58.0),
            Kind::Panel,
            18.0,
        ));
        elements.push(element(
            format!("order {i}"),
            (28.0, top + 6.0),
            (216.0, 26.0),
            Kind::Text(String::new()),
            20.0,
        ));
        elements.push(element(
            format!("patience {i}"),
            (28.0, top + 36.0),
            (216.0, 12.0),
            Kind::Bar,
            18.0,
        ));
    }
    Layout { elements }
}

/// The bell over the window rings: its graph (animators/window.ron) is told
/// `serve`, and plays its clip on the things under the window.
pub fn ring(world: &mut World) {
    for (moving, animates) in
        world.query_mut::<(&mut scrap::motion::Moving, &scrap::motion::Animates)>()
    {
        if animates.graph == "window" {
            moving.controller.trigger("serve");
        }
    }
    // And stars fly up from the window.
    for (mark, emitting) in
        world.query_mut::<(&crate::components::Mark, &mut scrap::particles::Emitting)>()
    {
        if mark.name == "serve fx" {
            emitting.emit(36);
        }
    }
}

/// What a cook holds, in that cook's hands as this peer has the cook: run
/// after the party has brought in the host's picture of the items, so the
/// player walking a cook sees what it carries go with it, not behind.
pub fn hold(world: &mut World) {
    let held: Vec<(Entity, u32)> = world
        .query::<(Entity, &crate::components::HeldBy)>()
        .iter()
        .map(|(e, h)| (e, h.0))
        .collect();
    for (item, index) in held {
        let Some(spot) = crate::state::cook(world, index)
            .and_then(|c| world.get::<&scrap::Transform>(c).ok().map(|t| *t))
            .map(|t| t.position + crate::state::facing(&t) * 0.55 + scrap::glam::Vec3::Y * 0.45)
        else {
            continue;
        };
        if let Ok(mut t) = world.get::<&mut scrap::Transform>(item) {
            t.position = spot;
        }
    }
}
