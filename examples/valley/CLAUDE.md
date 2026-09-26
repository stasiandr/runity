# valley

The engine's example project, laid out the way `scrap new` lays one out
(docs/layout.md): what a file is, its extension says, and where it lies
is by feature.

```
scrap.ron        the project file
budgets.ron      each scene's frame budget (`scrap perf`)
content/valley/
  maps/          the scenes, *.scene.ron — one entity per block, `id` first
  core/          what everything stands on: the atlas the models share, grid, white
  nature/        trees, plants, ground, rocks: each model beside its material
  camp/          the campfire (model, prefab, material, its embers) and the camp's pieces
  settlers/      the people
  water/, lava/  a material beside its own shader
  marks/         the decals: texture and material
  crafting/      the crafting tables (docs/data.md)
  CREDITS.txt    where every model came from (Kenney, CC0)
library/         built assets — derived, never committed
```

The engine's tests render these scenes, so a change here is a change to
what the tests check. `maps/first-light.scene.ron` uses builtins only and
opens with no library at all; keep it that way.

* Every entity has an `id`: sixteen hex digits, unique in its file. When
  writing one by hand, leave it out and the engine assigns one on load;
  never copy an existing one.
* Every source — model, texture, material — has its `.scrimport` beside
  it, committed. After adding or changing one, run
  `cargo run -p scrap-cli -- sync examples/valley` and commit what it
  writes; a test fails when a sidecar is missing or stale.
* After editing a scene or prefab, `cargo run -p scrap-cli -- check
  examples/valley` says whether every model, material and prefab it names
  exists, and which one it meant if not. CI runs it.
* Unlike a generated project this one has no `.gitattributes` of its own:
  it lives inside the engine's repository, which does not use Git LFS, and
  whose root `.gitattributes` keeps text LF.
