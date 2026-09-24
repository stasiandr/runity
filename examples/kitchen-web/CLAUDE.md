# kitchen

A scrap project. The layout is fixed — every tool, and every agent, finds
things in the same place:

```
scrap.ron   the project file
input.ron    actions by name ("jump"), and the keys for each
configs/     the game's data, RON, typed in code with scrap::Tuned
ui/          the game's screens: elements anchored in a 1280x720 frame (scrap::screen)
strings/     the game's words, one file per language; a screen says `@key`
dialogues/   conversations: lines, answers and the flags they set (scrap::dialogue)
animators/   which animation plays when: states and transitions, RON (scrap::animgraph)
clips/       clips that move things, not bones — a line's `animator` plays them (scrap::motion)
shaders/     materials' own looks: one WGSL `surface` function a file
layers.ron   collision layers, and which pairs pass through each other
scenes/      scenes, RON — one entity per block, `id` first
prefabs/     one entity subtree per file; a scene places it with `prefab: "name"`
materials/   .scrmat sources: `(color: "#rrggbb")`, sRGB hex
assets/      models, textures, sounds; each source gets a .scrimport beside it
library/     built assets — derived, never committed
Cargo.toml   the game crate; build.rs finds its components and systems
src/main.rs  the game: the window, and `step`, which runs the systems in order
src/components/  one component per file; the file name is the scene's name for it
src/systems/     one system per file: `pub fn run(world, seconds)`
```

* `scrap run` (or `cargo run`) plays `scenes/main.ron`. It keeps running
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
  beside it with the scenes, prefabs and built library — no sources.
* `scrap rename FROM TO` renames or moves a model, texture, sound,
  material or prefab, its .scrimport with it, and rewrites every scene and
  prefab line that named it. Scenes name assets by file stem, so renaming
  a file by hand breaks them; use this. `scrap uses FILE` lists those
  lines first. `scrap assets` lists every asset and how much it is used;
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
  a scene carries `SceneId`. `scrap add component door` writes
  `src/components/door.rs` (struct `Door`), and build.rs registers it as
  `"door"` — a scene line then gives it:
  `components: { "door": (locked: true) }`. A component the other
  players must see derives `Serialize` too and says
  `pub const NETWORKED: bool = true;` in its file: whoever owns the entity
  sends it. `scrap add system patrol`
  writes `src/systems/patrol.rs` and adds its call last in `step`; move the
  line to change the order. Never register components by hand or keep a
  list of them: the folder is the list. `scrap check` reports a component
  name no file answers to. Editing a value while the game runs changes that
  component and nothing else.
* Everything a person makes is text and is committed; `library/` and
  `target/` are not.
* Binary sources are in Git LFS and lockable — lock before editing one.
