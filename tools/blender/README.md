# runity for Blender

Build runity scenes in Blender (docs/blender.md). Keep a `.blend` in a
project's `assets/` and save: the engine imports it by itself.

* **The level** — everything in the file's scene — becomes a prefab named
  after the file. Put it in a runity scene with one line:
  `(name: "forest", prefab: "forest")`.
* **Mark as Asset** on a collection or an object makes it a prefab of its
  own, by its name. Its instances — Alt+D, collection instances, links from
  another `.blend` — stay instances of it.
* **Linked duplicates** (Alt+D) share a mesh, and so one model in the
  engine, drawn as instances. An object with modifiers gets its own.
* **Materials**: Principled BSDF becomes the engine's URP Lit — colour,
  metallic, roughness, emission, alpha, normal map, image textures. Swap one
  for a material of the project's in the file's `.rimport`:
  `materials: { "moss": "moss_wet" }`.
* **Components**: a custom property `runity.door` = `(open_angle: 90.0)`
  on an object is the game's component `door`.
* **Colliders**: an object named `…-col` is a static body that collides as
  its own shape.

## Install

Blender 4.2 or newer. Zip this `runity` folder and install it from
Edit › Preferences › Get Extensions › Install from Disk. On every save it
gives objects, meshes, materials and collections an ID (`runity.id`) that
survives renames, so what a runity scene says about a part keeps landing on
it. Without the add-on the file still imports, with IDs from object names.

The engine finds Blender in `RUNITY_BLENDER`, `/Applications/Blender.app`,
or `blender` on the `PATH`, and runs `export.py` from here in the
background to read a `.blend`.
