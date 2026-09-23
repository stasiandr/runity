// From Assets/Content/Art/Materials/Energy_Cell_Shader.shadergraph (URP Lit,
// opaque, both faces). Values from M_Energy_Cell_01.mat.
//
// The original colours the cell with _Color mixed 10% towards a sand
// texture (T_Huge_Sand_01), emits that colour x4, and pushes the vertices
// by a scrolling noise texture (_Displacement 0.01).
//
// This port: the sand texture is procedural value noise in sand colours;
// the vertex displacement is dropped (there is no vertex hook).

const ENERGY_CELL_COLOR: vec3<f32> = vec3<f32>(1.0, 0.3313, 0.0673);
const ENERGY_CELL_COLOR_INT: f32 = 0.1;
const ENERGY_CELL_EMISSIVE_INT: f32 = 4.0;
const ENERGY_CELL_SAND_DARK: vec3<f32> = vec3<f32>(0.22, 0.10, 0.035);
const ENERGY_CELL_SAND_LIGHT: vec3<f32> = vec3<f32>(0.75, 0.48, 0.24);

fn energy_cell_hash(p: vec2<f32>) -> f32 {
    let q = vec2<u32>(vec2<i32>(floor(p)) + vec2<i32>(32768));
    var h = q.x * 1597334677u ^ q.y * 3812015801u;
    h = h * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h) / 4294967295.0;
}

fn energy_cell_value(p: vec2<f32>) -> f32 {
    let i = floor(p);
    var f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    let a = energy_cell_hash(i);
    let b = energy_cell_hash(i + vec2<f32>(1.0, 0.0));
    let c = energy_cell_hash(i + vec2<f32>(0.0, 1.0));
    let d = energy_cell_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn energy_cell_sand(uv: vec2<f32>) -> vec3<f32> {
    var t = 0.0;
    var amp = 0.5;
    var p = uv * 12.0;
    for (var i = 0; i < 4; i = i + 1) {
        t = t + energy_cell_value(p) * amp;
        p = p * 2.03;
        amp = amp * 0.5;
    }
    return mix(ENERGY_CELL_SAND_DARK, ENERGY_CELL_SAND_LIGHT, clamp(t * 1.2 - 0.1, 0.0, 1.0));
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let base = mix(ENERGY_CELL_COLOR, energy_cell_sand(in.uv), ENERGY_CELL_COLOR_INT);
    o.albedo = base;
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = base * ENERGY_CELL_EMISSIVE_INT;
    return o;
}
