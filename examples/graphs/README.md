# Graphs — a gallery of shader and effect graphs

Every look in this project is a graph — `content/graphs/<feature>/`, each
beside its material and textures — text you can read, diff and edit while
the game runs (docs/shadergraph.md):

| On the pedestals, left to right | Graph | What it shows |
|---|---|---|
| hologram | `hologram.graph.ron` | Fresnel rim, lines up the screen, a ripple from a subgraph, transparency |
| dissolve | `dissolve.graph.ron` | noise cut out with `clip` as time goes, a glowing edge, a `Gradient` |
| toon | `toon.graph.ron` | the sun's light (`light_direction`, `light_color`) in flat bands |
| gold | `gold.graph.ron` | Unity's Specular workflow (`surface: (specular: …)`), a hammered normal |
| pixel art | `pixel.graph.ron` | `Texture(filter: Point, wrap: Mirror)`, a gleam of `Environment` |
| rock | `rock.graph.ron` | `Triplanar`, a coarser mip level (`lod`) far off, moss on top |
| marble | `marble.graph.ron` | `Twirl`, `Gradient`, `Blend(mode: Overlay)`, `Hue`, `Contrast` |

Around them:

* **banner** (`banner.graph.ron`) — waved by the vertex stage; its colours
  and border are the material's typed properties (`Color`, `Boolean`).
* **pool** (`water.graph.ron`) — raised by the vertex stage, what is under
  it seen bent (`SceneColor`), foam where it is shallow (`scene_depth`),
  the sky at a glance (`Fresnel` over `Environment`); its drops are the
  `ripple` subgraph the hologram uses too.
* **fountain** (`fountain.vfx.ron`) — GPU sparks whose colour and lift are
  the emitter's `params`, each cooling by a heat of its own (`custom`).
* **fireflies** (`fireflies.vfx.ron`) — born anywhere in the emitter's box,
  drifting on `Turbulence`, blinking on a phase of their own.
* **sparks** (`sparks.vfx.ron`) — drawn with their material's picture, a
  sheet of four shapes, the graph picking each one's (`frame`).
* **the whole picture** (`film.post.ron`, the scene's `fullscreen`) —
  distance haze from the picture's depth, darkened corners, grain.

```
scrap check examples/graphs          # builds every graph, says what is wrong
scrap run examples/graphs            # the gallery; save a graph, see it change
scrap build examples/graphs          # a folder to ship, in examples/graphs/build
```

In the studio, Window › Shader Graph shows each graph as boxes and
arrows with previews; an agent has the same through the MCP tools
`shader_graph`, `shader_graph_edit` and `shader_graph_preview`.

The shared pieces — the `ripple` subgraph, the fullscreen `film` graph,
the pedestals' material — are in `content/graphs/core/`, the scene in
`content/graphs/maps/main.scene.ron`. The textures were drawn for this
example.
