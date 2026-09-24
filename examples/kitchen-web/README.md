# Kitchen Rush — in the browser

The kitchen template (`examples/kitchen`, made with
`runity new --template kitchen`) built for the web: the engine on WebGPU,
sticks and buttons on the screen for a phone, and a room code to cook
together. Published to GitHub Pages by `.github/workflows/pages.yml`.

- **Soup**: three chopped tomatoes (or onions) in a pot, cooked, onto a plate.
- **Salad**: chopped tomato and chopped cabbage, put together on a plate.
- **Burger**: a bun, a patty fried on a pan, chopped cabbage — on a plate.

The rules, the scenes and the screens are the template's; its README is the
map of where each part of the engine is used. What is different here:

| What | Where |
|---|---|
| The page: loading, errors, the room badge, the join dialog | `web/index.html` |
| Sticks and buttons on a touch screen, as a pad | `web/pad.js` → `runity_pad_axis` / `runity_pad_button` (`runity-shell`'s `web`) |
| Rooms: a code and a link, WebRTC data channels through PeerJS's broker | `web/runity-net.js`, `src/lobby.rs`; the wire is `runity::net::page::Page` |
| The project's data as one gzipped file, mounted where the game reads it | `web/pack.py`, `src/web.rs` → `runity::files::mount` |
| Quality: Low on a phone, by the GPU elsewhere, `?quality=` to choose | `src/web.rs`, `runity::quality` |
| The player's prefs and best score, kept in the browser | `runity::files::on_write` → `localStorage` |

## Building

```
runity sync                 # the library, from assets/ (once, and after changing one)
web/build.sh                # → site/: the game, data.bin.gz, the page
python3 -m http.server -d site 8765
```

Needs the `wasm32-unknown-unknown` target and `wasm-bindgen-cli` at the
version in `Cargo.lock`; `wasm-opt` is used when it is there. `cargo run`
still opens the kitchen in a desktop window, alone.

## Playing together

**Host a kitchen** opens a room and shows its code at the top; **Invite
friends** shares the link (the phone's share sheet, or the clipboard). A
friend opens the link — or presses **Join a friend** and types the code — and
is in the kitchen. The host's **Open the doors** starts the round for all.

The host's browser runs the session (the engine's server, unthreaded, ticks
inside the party); guests talk to it over one WebRTC data channel each, peer
to peer. PeerJS's public broker only introduces them. Two browsers behind
NATs that STUN cannot get through need a TURN relay, and the free one PeerJS
offers may be down: then joining fails after 20 s with "no kitchen answered".

The host's tab has to stay in front: a browser stops drawing a hidden page,
and the kitchen stops with it.

## On a phone

Hold it sideways. The left thumb walks (the stick comes to where it lands),
the right one grabs, chops, throws and points; ⏸ pauses. Needs WebGPU: Safari
on iOS 26, Chrome on Android.

## Assets

The models, sprites and font are [Kenney](https://kenney.nl)'s, CC0 —
`assets/kenney/LICENSE-kenney.txt`.
