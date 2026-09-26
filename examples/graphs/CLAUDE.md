# graphs

A scrap project. What a file is, its extension says — not the folder it
lies in — so every tool, and every agent, finds a file wherever it is
(docs/layout.md in the engine). Keep what belongs together together: a
tomato's model, texture, material and prefab in one folder; code by
feature. The layout `scrap new` wrote is Unreal's:

```
scrap.ron     the project file: name, modules, the start scene by name
config/       settings for the whole game
  input.ron   actions by name ("jump"), and the keys for each
  layers.ron  collision layers, and which pairs pass through each other
content/      everything a person makes
  graphs/   the game's own folder
    maps/     levels
    core/     what everything else stands on; world.ron, the game's numbers
    props/    a feature: its model, texture, material and prefab together
    ui/screens/     the screens
    ui/components/  pieces of screens
  localization/     the game's words, one file per language; a screen says `@key`
  developers/ sandboxes: the editor reads them, `scrap build` does not ship them
library/      built assets — derived, never committed
Cargo.toml    the game crate; build.rs finds its components and systems
src/main.rs   the game: the window, and `step`, which runs the systems in order
src/<feature>/  code by feature: `spin/spin.rs` is the component `spin`, `spin/turn.rs` a system
```

Kinds, by extension — find them anywhere with a glob (`**/*.prefab`):

```
*.scene.ron     scenes, RON — one entity per block, `id` first
*.prefab        one entity subtree per file; a scene places it with `prefab: "name"`
*.scrmat        materials: `(color: "#rrggbb")`, sRGB hex
*.screen.ron    the game's screens: elements anchored in a 1280x720 frame (scrap::screen)
*.animator.ron  which animation plays when: states and transitions (scrap::animgraph)
*.clip.ron      clips that move things, not bones — a line's `animator` plays them (scrap::motion)
*.dialogue.ron  conversations: lines, answers and the flags they set (scrap::dialogue)
*.quest.ron     rows of stages done by the dialogues' flags (scrap::quest)
*.cases.ron     beside a graph or a dialogue: the cases `scrap check` plays
*.wgsl, *.graph.ron  materials' own looks: `shader: "water"` is water.wgsl
*.vfx.ron       particle effect graphs
*.post.ron      fullscreen graphs, over the whole picture: a scene says `fullscreen: (graph: "name")`
*.ron           anything else is the game's data: one struct is scrap::Tuned, records by name scrap::Table
models, textures, sounds  sources; each gets a .scrimport beside it
```

A name is the file's name without that extension: `maps/cave.scene.ron`
is the scene `cave`, wherever it lies. Two files of one kind with one name
are ambiguous to a line that names only the name; `scrap check` says so.

* `scrap run` (or `cargo run`) plays the scene `main`. It keeps running
  while you edit: saved scenes, prefabs and rebuilt assets show up in the
  window. `scrap run --hot` patches the game's own Rust in too, under
  `dx serve --hotpatch` (`cargo install dioxus-cli`).
* `scrap check` says what does not resolve — a model, a material or a
  prefab nobody has, a repeated id, a stale sidecar — with the file and the
  entity. Run it after editing scenes; it exits non-zero on errors.
* `scrap git-setup`, once per clone, turns on the merge driver: scenes
  and prefabs then merge by entity and field, and a real conflict is
  reported in words ("both changed the position of `tree`") with ours
  kept and the file still loading.
* `scrap build` makes a folder to ship: the game in release, and `data/`
  beside it with the scenes, prefabs, screens, data and built library at
  the same paths — no sources, and nothing from `developers/`.
* Scenes link assets by ID and name (`model: ("rock", "3f9a…")`), so
  a file moved to another folder — with its `.scrimport` beside it — is
  still found. `scrap rename FROM TO` renames or moves a model, texture,
  sound, material or prefab and its .scrimport together, and freshens the
  name in every line that named it; `scrap uses FILE` lists those lines
  first. `scrap assets` lists every asset and how much it is used;
  `scrap delete FILE` removes one only when nothing names it;
  `scrap duplicate FROM TO` copies one as a new asset.
* `scrap sync` builds `library/` from the sources. After adding, changing
  or moving a source, run it and commit the `.scrimport` it writes beside the
  source. Never edit a sidecar's `hash` or `id` by hand: the hash is how a
  moved file is found, the id is what the library refers to.
* Every entity has an `id`: sixteen hex digits, unique in its file. When
  writing one by hand, leave it out and the engine assigns one on load;
  never copy an existing one.
* The editor is also an MCP server: with scrap's `scrap-mcp` on the PATH
  (`cargo install --path crates/scrap-mcp` in the engine),
  `claude mcp add scrap -- scrap-mcp` gives every editor operation as a
  tool — open, add, move, undo, render a frame to look at, check, simulate.
* Game logic is Rust on the ECS (`scrap::hecs`): components are plain
  structs, systems are functions over the world, and an entity spawned from
  a scene carries `SceneId`. `scrap add component doors/door` writes
  `src/doors/door.rs` (struct `Door`), and build.rs registers it as
  `"door"` — a scene line then gives it:
  `components: { "door": (locked: true) }`. A component the other
  players must see derives `Serialize` too and says
  `pub const NETWORKED: bool = true;` in its file: whoever owns the entity
  sends it. `scrap add system guards/patrol`
  writes `src/guards/patrol.rs` and adds its call last in `step`; move the
  line to change the order. build.rs finds them in every folder under
  `src/`: a file declaring the struct of its name is a component, one with
  `pub fn run(` a system; the files right in `src/`, and folders with a
  `mod.rs`, are the game's own modules and are left alone — so never
  declare a component's file with `mod` yourself. Never register
  components by hand or keep a list of them: the files are the list.
  `scrap check` reports a component name no file answers to. Editing a value while the game runs changes that
  component and nothing else.
* The player's size is `game: (player: (height, radius, step, slope,
  jump_height, speed, gravity))` in `scrap.ron`: navigation bakes for it,
  the editor draws it (View › Player), and a controller should read the
  same numbers (`scrap::project::GameSettings::load`). The line the player
  starts as — or an empty the game spawns it at — says
  `player_start: true`; Play › Play from Here moves it where the editor
  looks (`LiveScene::start_here` in `start`).
* Everything a person makes is text and is committed; `library/` and
  `target/` are not.
* Binary sources are in Git LFS and lockable — lock before editing one.
