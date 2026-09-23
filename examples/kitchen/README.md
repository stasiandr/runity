# Kitchen Rush

A little Overcooked on runity: two to four cooks, one kitchen, orders for
soup coming in faster than is comfortable. Chop tomatoes or onions on a
board, three of one kind into a pot, the soup onto a plate, the plate out
of the window before the order walks out.

It is also the engine's worked example: nearly everything runity does is
used here once, in the place a game would use it. Read it as a map.

```
cargo run --release                       # the menu: cook here, host, join
runity run --players 2                    # two windows playing together
runity run --players 2 --link poor        # …over a bad link, on purpose
cargo test                                # the rules, the network, screenshots
runity check                              # every name the files use resolves
```

Keys: cook 1 is WASD, E to grab, F to chop (and scrape a burnt pot), Q to
throw food; cook 2 is the arrows, Enter, Right Shift and Right Ctrl. A pad
drives cook 1. Escape pauses.

## Where each part of the engine is

| What | Where |
|---|---|
| **Scene** as text, one line an entity, stable ids | `scenes/main.ron` — every counter a line with a `station` |
| **Prefabs** spawned at run time | `prefabs/tomato.prefab`, `onion`, `plate`; `session::spawns` |
| **Components** as files, **systems** in order | `src/components/`, `src/systems/`, `tick` in `src/main.rs` |
| **Networking**: host, join, ownership, replication | `front::seat`/`claim` (who plays which cook), `session::act` (what hands did, checked on the host) |
| Networked components | `pub const NETWORKED: bool = true;` in `pot.rs`, `chop.rs`, `served.rs`, `round.rs`, `seat.rs`, `held_by.rs` (what a cook holds shows in their hands at once, `front::hold`) |
| One-off messages (`Party::publish`) | `state::Act`, sent by `Front::drive` |
| Spawns everyone sees (`Party::spawn`) | `session::spawns` |
| **Screens** (UI Builder files) | `ui/menu.ron`, `hud.ron`, `results.ron`, `pause.ron`, `speech.ron` |
| Buttons, fields, sliders, a choice | the menu: host/join, your name and the address, volumes, language |
| Screens built in code | the order cards, `front::order_cards` |
| **A screen in the world** (`WorldUi`) | the order board on the back wall, `systems/board.rs` |
| **Localisation** | `strings/en.ron`, `strings/ru.ron`; every `@key` checked by a test |
| **Dialogue** | `dialogues/chef.ron`, the head chef before the doors open |
| **Sound**: sources in the scene, groups, one-shots | the stoves' `sound` (boiling, turned up as the soup cooks), the kitchen's music, `noise.rs` for the rest |
| **Particles** | the steam off a done soup and the smoke off a burnt one (`steam`, `smoke` under each stove) |
| **Things moved by a graph** (`animator`, `clips/`) | the bell over the window rings on every soup served: `animators/window.ron`, `clips/bell_ring.ron`, `front::ring` |
| **Physics** (rapier): bodies made at run time, a first speed | thrown food: `hands::throw`, `launch` in `src/main.rs`, `systems/fly.rs`; the counters, the floor and the wall are static bodies in the scene |
| **Routes** | the customers walking past outside |
| **Decals** | the stains on the floor |
| **Lights**, **post-processing** | a warm lamp over the stoves, the vignette (`post:` in the scene) |
| Switched-off things (`inactive`) | the soup, bars and steam marks; cooks nobody plays |
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
