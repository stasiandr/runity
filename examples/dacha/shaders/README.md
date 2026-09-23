# Dacha Simulator's shaders, written again

Unity Shader Graphs and `.shader` files from `~/personal/dacha-simulator`,
each a `surface` function over runity's standard shader (see
`crates/runity/src/render.wgsl`, `runity:surface`). Each file's header says
where it came from and what it leaves out — mostly vertex displacement, the
scene behind a surface (refraction, depth fades), and extra textures, which
are procedural noise here.

`runity import-unity ~/personal/dacha-simulator PROJECT --shaders
examples/dacha/shaders` puts these in the project's `shaders/` in place of
the stubs the importer writes. Check one with `cargo run -p runity --example
check_surface -- FILE`, look at them all with `--example shader_gallery`.

Not here yet: `mirror` (a camera's picture: `render_texture`), `outline`
(a second pass), the screen and UI ones (`screen_*`, `ui_*`: overlays, not
surfaces) and TextMesh Pro's.

Where a shader is shared by materials with different settings, the values
that differ come from each material: the file's `// runity:params` line
names them and the importer fills them in from each `.mat`.
