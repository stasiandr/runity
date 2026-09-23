// Huge sand: the far dunes.
// From Assets/Content/Art/Materials/Huge_Sand_Shader.shadergraph (M_Huge_Sand_01).
// The original blends the flat colour _Color towards the T_Huge_Sand_01
// texture (by _Color_Int = 0.36, on the mesh's UVs), and in the vertex stage
// lifts the mesh by a slowly scrolling noise texture.
// Here the sand texture is procedural dune ridges on the same UVs. The vertex
// lift is left out: a surface function cannot move vertices. Smoothness,
// metallic and emission are the graph's own (all zero), not the .mat's.

const HUGE_SAND_COLOR = vec3<f32>(0.2946, 0.149, 0.0925);
const HUGE_SAND_COLOR_INT = 0.36;
const HUGE_SAND_DARK = vec3<f32>(0.13, 0.055, 0.018);
const HUGE_SAND_LIGHT = vec3<f32>(0.76, 0.45, 0.19);
const HUGE_SAND_RIDGES = 8.0;

fn huge_sand_hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(123.34, 456.21));
    let r = q + dot(q, q + 45.32);
    return fract(r.x * r.y);
}

fn huge_sand_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = huge_sand_hash(i);
    let b = huge_sand_hash(i + vec2<f32>(1.0, 0.0));
    let c = huge_sand_hash(i + vec2<f32>(0.0, 1.0));
    let d = huge_sand_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn huge_sand_fbm(p: vec2<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 4; i = i + 1) {
        sum = sum + amp * huge_sand_noise(q);
        q = q * 2.03 + vec2<f32>(17.1, 9.3);
        amp = amp * 0.5;
    }
    return sum;
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let p = in.uv * HUGE_SAND_RIDGES;
    let warp = huge_sand_fbm(p * 0.7) * 2.2;
    // Ridges: a steep lit face and a long shaded one, bent by the warp.
    let phase = fract(p.x * 0.45 + p.y + warp);
    let ridge = smoothstep(0.0, 0.25, phase) * (1.0 - smoothstep(0.35, 1.0, phase));
    let grain = huge_sand_fbm(p * 6.0);
    let t = clamp(ridge * 0.75 + grain * 0.4 - 0.05, 0.0, 1.0);
    let sand = mix(HUGE_SAND_DARK, HUGE_SAND_LIGHT, t);
    o.albedo = mix(HUGE_SAND_COLOR, sand, HUGE_SAND_COLOR_INT);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = vec3<f32>(0.0);
    return o;
}
