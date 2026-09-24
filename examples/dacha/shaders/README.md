# Dacha Simulator's shaders, written again

Unity Shader Graphs and `.shader` files from `~/personal/dacha-simulator`,
each a `surface` function over runity's standard shader (see
`crates/runity/src/render.wgsl`, `runity:surface`). Each file's header says
where it came from and what it leaves out — mostly vertex displacement and
the scene behind a surface (refraction, depth fades). A graph that reads
more than four textures keeps the rest as procedural noise.

`runity import-unity ~/personal/dacha-simulator PROJECT --shaders
examples/dacha/shaders` puts these in the project's `shaders/` in place of
the stubs the importer writes. Check one with `cargo run -p runity --example
check_surface -- FILE`, look at them all with `--example shader_gallery`.

A shader here can say what its materials need, on lines of its own:
`// runity:params _Speed _Tint.r` (the material's eight numbers, from its
`.mat`), `// runity:base_map render:mirror` and `// runity:screen_map
Mirror` — see `mirror.wgsl`, the planar mirror. `// runity:textures _Road
_Noise` names up to four of the material's textures, by the property names
its `.mat` gives them (a texture set in a Sample Texture 2D node is
`_SampleTexture2D_<node>_Texture_1_Texture2D`), for `texture_at(in, 0u, uv)`
and on; the importer copies every texture the graph reads into the
`.rmat`'s `textures`.

Not here yet: `outline`
(a second pass), the screen and UI ones (`screen_*`, `ui_*`: overlays, not
surfaces) and TextMesh Pro's.

Where a shader is shared by materials with different settings, the values
that differ come from each material: the file's `// runity:params` line
names them and the importer fills them in from each `.mat`.
