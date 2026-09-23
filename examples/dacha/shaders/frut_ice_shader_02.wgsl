// From Assets/Content/Art/Materials/Frut_Ice_Shader_02.shadergraph, values
// from MI_Frut_Ice_02.mat (M_Water_01.mat uses it too, with _Ice_Int 0.2 and a
// paler _Ice_Color).
//
// The original is an unlit transparent graph: alpha is a frost texture
// (T_Freeze__01_M) plus _Ice_Int; colour is _Ice_Color plus glints where a
// sparse dot texture (T_Noise_Glimmer_01_M) is bright both in the mesh's UVs
// and in screen space, faded by a hard-contrast scene-depth term.
//
// Both textures are replaced by procedural stand-ins: frost by random-valued
// Voronoi shards, glimmer by hashed dots on a 512 grid. Scene depth (what is
// behind the ice) is replaced by the fragment's own distance with the
// camera's 1000 m far plane, and clamped at 0 (the original goes negative
// there, darkening far glints). Unlit is imitated with a black albedo and the
// colour in `emission`. MI_Frut_Ice_02.mat has no _Surface, so the importer
// may bring it in as opaque and ignore `alpha`.

const FRUT_ICE_SHADER_02_ICE_COLOR = vec3<f32>(0.30113026, 0.7535945, 0.95283014);
const FRUT_ICE_SHADER_02_ICE_INT = 0.0;
const FRUT_ICE_SHADER_02_ICE_TILING = 1.0;
const FRUT_ICE_SHADER_02_FAR = 1000.0;
const FRUT_ICE_SHADER_02_GLIMMER_TEXELS = 512.0;

fn frut_ice_shader_02_hash(p: vec2<f32>) -> f32 {
    var h = vec2<u32>(vec2<i32>(p));
    var x = h.x * 1664525u + h.y * 1013904223u + 374761393u;
    x = (x ^ (x >> 15u)) * 2246822519u;
    x = (x ^ (x >> 13u)) * 3266489917u;
    x = x ^ (x >> 16u);
    return f32(x) / 4294967295.0;
}

// Frost: the value of the nearest of a jittered grid of cells, with a
// finer layer over it, reading like the shattered-crystal texture.
fn frut_ice_shader_02_shards(uv: vec2<f32>) -> f32 {
    let g = floor(uv);
    let f = fract(uv);
    var best = 8.0;
    var value = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let cell = g + vec2<f32>(f32(x), f32(y));
            let jitter = vec2<f32>(
                frut_ice_shader_02_hash(cell + vec2<f32>(1000.0, 0.0)),
                frut_ice_shader_02_hash(cell + vec2<f32>(0.0, 1000.0)),
            );
            let d = length(vec2<f32>(f32(x), f32(y)) + jitter - f);
            if d < best {
                best = d;
                value = frut_ice_shader_02_hash(cell);
            }
        }
    }
    return value;
}

fn frut_ice_shader_02_frost(uv: vec2<f32>) -> f32 {
    let coarse = frut_ice_shader_02_shards(uv * 24.0);
    let fine = frut_ice_shader_02_shards(uv * 70.0 + vec2<f32>(17.0, 31.0));
    return mix(0.3, 1.0, coarse * 0.6 + fine * 0.4);
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;

    let frost = frut_ice_shader_02_frost(in.uv * FRUT_ICE_SHADER_02_ICE_TILING);
    o.alpha = clamp(frost + FRUT_ICE_SHADER_02_ICE_INT, 0.0, 1.0);

    // Glimmer in UV: step(1, r * 3).
    let uv_dot = frut_ice_shader_02_hash(floor(in.uv * FRUT_ICE_SHADER_02_GLIMMER_TEXELS));
    let uv_glint = step(0.92, uv_dot);
    // Glimmer in screen space at 0.71 tiling: step(1, r).
    let clip = frame.view_projection * vec4<f32>(in.world_position, 1.0);
    let screen = clip.xy / clip.w * 0.5 + 0.5;
    let screen_dot = frut_ice_shader_02_hash(floor(screen * 0.71 * FRUT_ICE_SHADER_02_GLIMMER_TEXELS) + vec2<f32>(5000.0, 7000.0));
    let screen_glint = step(0.96, screen_dot);

    // 1 - Contrast(Linear01Depth * 100, 100).
    let depth01 = length(frame.camera_position.xyz - in.world_position) / FRUT_ICE_SHADER_02_FAR;
    let midpoint = pow(0.5, 2.2);
    let near_fade = max(1.0 - ((depth01 * 100.0 - midpoint) * 100.0 + midpoint), 0.0);

    let color = FRUT_ICE_SHADER_02_ICE_COLOR + vec3<f32>(uv_glint * screen_glint * near_fade);
    o.albedo = vec3<f32>(0.0);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = color;
    return o;
}
