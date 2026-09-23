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
* Unlike a generated project this one has no `.gitattributes`: it lives
  inside the engine's repository, which does not use Git LFS.
