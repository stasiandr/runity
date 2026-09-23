# valley

The engine's example project, laid out the way every runity project is:

```
runity.ron   the project file
scenes/      scenes, RON — one entity per block, `id` first
prefabs/     one entity subtree per file; a scene places it with `prefab: "name"`
materials/   .rmat sources: `(color: "#rrggbb")`, sRGB hex
assets/      models and textures (Kenney, CC0 — see assets/CREDITS.txt)
library/     built assets — derived, never committed
```

The engine's tests render these scenes, so a change here is a change to
what the tests check. `scenes/first-light.ron` uses builtins only and opens
with no library at all; keep it that way.

* Every entity has an `id`: sixteen hex digits, unique in its file. When
  writing one by hand, leave it out and the engine assigns one on load;
  never copy an existing one.
* Every source in `assets/` and `materials/` has its `.rimport` beside it,
  committed. After adding or changing one, run
  `cargo run -p runity-cli -- sync examples/valley` and commit what it
  writes; a test fails when a sidecar is missing or stale.
* After editing a scene or prefab, `cargo run -p runity-cli -- check
  examples/valley` says whether every model, material and prefab it names
  exists, and which one it meant if not. CI runs it.
* Unlike a generated project this one has no `.gitattributes` of its own:
  it lives inside the engine's repository, which does not use Git LFS, and
  whose root `.gitattributes` keeps text LF.
