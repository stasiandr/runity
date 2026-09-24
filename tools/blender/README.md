# scrap for Blender

Build scrap scenes in Blender (docs/blender.md). Keep a `.blend` in a
project's `assets/` and save: the engine imports it by itself.

* **The level** — everything in the file's scene — becomes a prefab named
  after the file. Put it in a scrap scene with one line:
  `(name: "forest", prefab: "forest")`.
* **Mark as Asset** on a collection or an object makes it a prefab of its
  own, by its name. Its instances — Alt+D, collection instances, links from
  another `.blend` — stay instances of it.
* **Linked duplicates** (Alt+D) share a mesh, and so one model in the
  engine, drawn as instances. An object with modifiers gets its own.
* **Materials**: Principled BSDF becomes the engine's URP Lit — colour,
  metallic, roughness, emission, alpha, normal map, image textures. Swap one
  for a material of the project's in the file's `.scrimport`:
  `materials: { "moss": "moss_wet" }`.
* **The scrap tab** in the 3D view's sidebar (N) shows the active
  object's game settings:
  * **Add Component** lists the game's components. Each value gets a field
    of its own: a number is a number field, a bool is a checkbox, text is
    text. They are stored as `scrap.door.open_angle`-style properties.
    A property `scrap.door` = `(open_angle: 90.0)` holding the whole value
    works too.
  * **Collider** makes the object a static body that collides as its own
    shape. Naming it `…-col` does the same.
  * **Game material** draws the object with a material of the project's
    instead of Blender's. The tab also shows which of the object's
    materials the project swaps file-wide in the `.scrimport`.
  * A line at the top says whether a scrap editor has the project open.
* **Live**: while the scrap editor has the project open, saving sends the
  file straight to it (no second Blender starts), and moving objects moves
  them there as you drag. The drag is a preview; the save is what counts.

The component list and the materials come from `library/blender.json`,
which the editor writes when it opens the project. `scrap.id`,
`scrap.collider` and `scrap.material` are the tab's own settings, not
components.

## Install

Blender 4.2 or newer. In the scrap editor: **Tools › Install Blender
Plugin**. It puts this folder into Blender's user extensions
(`user_default`), turns it on and saves Blender's preferences. Quit
Blender first: the editor refuses while one is open, because Blender saves
its preferences when it quits, over the ones that turn the plugin on. By hand: zip this
`scrap` folder and use Edit › Preferences › Get Extensions › Install from
Disk. On every save it
gives objects, meshes, materials and collections an ID (`scrap.id`) that
survives renames, so what a scrap scene says about a part keeps landing on
it. Without the add-on the file still imports, with IDs from object names.

The engine finds Blender in `SCRAP_BLENDER`, `/Applications/Blender.app`,
or `blender` on the `PATH`, and runs `export.py` from here in the
background to read a `.blend`.
