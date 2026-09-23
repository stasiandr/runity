# Kitchen Rush

A little Overcooked on runity, played together over Steam: up to four
cooks, one kitchen, orders coming in faster than is comfortable.

- **Soup**: three chopped tomatoes (or onions) in a pot, cooked, onto a
  plate.
- **Salad**: chopped tomato and chopped cabbage, put together on a plate.
- **Burger**: a bun, a patty fried on a pan, chopped cabbage — on a plate.

Plates go out of the window before the order walks out, and come back to
the sink dirty a few seconds later: someone has to wash them (hold F), or
the racks run dry. Pots and pans burn what is left on them.

It is also the engine's worked example: nearly everything runity does is
used here once, in the place a game would use it. Read it as a map.

```
cargo run --release                       # the menu: host a kitchen, join a friend
runity run --players 2                    # two windows playing together, over UDP
runity run --players 2 --link poor        # …over a bad link, on purpose
cargo test                                # the rules, the network, screenshots
cargo test -- --ignored steam             # a real Steam lobby (Steam running)
runity check                              # every name the files use resolves
```

With Steam running, **Host a kitchen** makes a lobby your friends can see
(**Join a friend** opens the Steam friends list to join theirs);
**Invite friends** in the lobby opens Steam's invite dialog, and a friend
who accepts — or joins your game from their friends list — is in. The
kitchen opens shut: everyone can walk about, and the host's **Open the
doors** starts the round for all. The app is Valve's Spacewar (480), which
every Steam account may run for testing (`src/lobby.rs`, `APP_ID`).
Without Steam, the kitchen is yours alone.

Keys: WASD (or the arrows, or the stick) to walk, E to grab, F to chop
(and scrape a burnt pot, and wash plates), Q to throw food. Escape pauses.

## Where each part of the engine is

| What | Where |
|---|---|
| **Scene** as text, one line an entity, stable ids | `scenes/main.ron` — every counter a line with a `station` |
| **Prefabs** spawned at run time | `prefabs/tomato.prefab`, `onion`, `plate`; `session::spawns` |
| **Components** as files, **systems** in order | `src/components/`, `src/systems/`, `tick` in `src/main.rs` |
| **Steam**: a lobby, invites, the session over Steam's networking | `src/lobby.rs` — `Steam::wire` is the transport the session runs on, the `Steam` itself stays with the game for the lobby |
| **Networking**: host, join, ownership, replication | `front::seat`/`claim` (who plays which cook), `session::act` (what hands did, checked on the host) |
| Networked components | `pub const NETWORKED: bool = true;` in `pot.rs`, `chop.rs`, `served.rs`, `round.rs`, `seat.rs`, `held_by.rs` (what a cook holds shows in their hands at once, `front::hold`) |
| One-off messages (`Party::publish`) | `state::Act`, sent by `Front::drive` |
| Spawns everyone sees (`Party::spawn`) | `session::spawns` |
| **Screens** (UI Builder files) | `ui/menu.ron`, `lobby.ron`, `hud.ron`, `results.ron`, `pause.ron`, `speech.ron` |
| Buttons, fields, sliders, a choice, a list, hidden elements | the menu (host, join, volumes, language; the address row only without Steam), the lobby's players and the start only the host sees (`Screen::set_hidden`) |
| Screens built in code | the order cards, `front::order_cards` |
| **A screen in the world** (`WorldUi`) | the order board on the back wall, `systems/board.rs` |
| **Localisation** | `strings/en.ron`, `strings/ru.ron`; every `@key` checked by a test |
| **Dialogue** | `dialogues/chef.ron`, the head chef before the doors open |
| **Sound**: sources in the scene, groups, one-shots | the stoves' `sound` (boiling, turned up as the soup cooks), the kitchen's music, `noise.rs` for the rest |
| **Particles** with sprites | steam off a done soup, smoke off a burnt one, the burner's flame, juice off the knife, stars when a soup goes out, dust from a running cook — `*_fx.rmat` over Kenney's particle sprites in `assets/fx/` |
| **Models**: glTF with their colours | Kenney's furniture, food and characters (`assets/kenney/`); a model's own colours come with it as its look, no material needed |
| **Skinned characters** blended by speed | the cooks and customers: `animators/cook.ron`, `walker.ron`, `systems/pose.rs` |
| **A font of the game's own**, a **widget style** | Kenney Future, `front::FONT`; round buttons with a lip, `front::style` |
| **Things moved by a graph** (`animator`, `clips/`) | the bell over the window rings on every soup served: `animators/window.ron`, `clips/bell_ring.ron`, `front::ring` |
| **Physics** (rapier): bodies made at run time, a first speed | thrown food: `hands::throw`, `launch` in `src/main.rs`, `systems/fly.rs`; the counters, the floor and the wall are static bodies in the scene |
| **Routes** | the customers walking past outside |
| **Decals** | the stains on the floor |
| **Lights**, **post-processing** | a warm lamp over the stoves; ACES, bloom, vignette, depth of field, grain, TAA and SSAO (`post:`, `ambient_occlusion:` in the scene) |
| Switched-off things (`inactive`) | the soup, bars and steam marks; cooks nobody plays |
| **A camera tour** (`runity::tour`), tuned from a file | the fly-through while players gather: the shots in `tuning/flyby.ron` (saved while running, it changes at once), eased in and out in `main.rs`; the lens focuses on what it looks at |
| **Words over the world** (`runity::floaters`) | points won rising over the window, lost at the board: `Front::watch_score` |
| State everyone follows | `Round::open`: shut, the kitchen is the lobby; the host opens it (`Act::Restart`) and every peer's screen follows |
| **Player prefs** | the volumes, the language and the best score, kept between runs |
| **Input** actions and axes, keyboard and pad | `input.ron` |
| Testing headless, together, and with pictures | `src/play_tests.rs`: a host and a guest over a loopback; `target/shots/*.png` |

## How the network is shared out

The kitchen is the host's. It runs the rules — the orders, the clock, who
holds what, what is cooking — and sends everyone what they need to see:
the pots, the chopping, the plates' soup and the round as networked
components, and every item's place as its transform. Each player walks
their own cook (they own it, so it moves at once on their screen) and
sends what their hands do as an act; the host does it, if it is theirs to
do. Sounds, the bell and the steam are worked out on every peer from what
changed, so they are never sent.

## Assets

The models, sprites and font are [Kenney](https://kenney.nl)'s, CC0 —
`assets/kenney/LICENSE-kenney.txt`. The particle sprites' colour was made
white, their shape left in the alpha, so a material's colour tints them.
