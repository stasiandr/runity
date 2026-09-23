// From Assets/Content/Art/Materials/Return_To_Shader_01.shadergraph (URP
// Unlit, transparent, both faces). Values from the M_Return.mat in
// _Incoming/Tools/SexyMouseTrap (the one the prefab uses; the M_Return.mat
// in Art/Materials has _ALL_INT 0 and draws nothing).
//
// A "rewind" overlay: the scene behind, read with a noise-jittered screen
// position and made grey, darkened towards the edges, with faint posterized
// noise lines rolling over it; alpha is _ALL_INT x a mask texture
// (T_SexyMouse2, a mouse-head silhouette).
//
// This port: the scene colour cannot be read, so a flat grey stands in for
// it and the jitter is dropped; the mask texture is a rough procedural
// mouse head (a disc and two ears). The .mat says _Surface: 0, so unless
// the material is made transparent the alpha has no effect.

const RETURN_TO_ALL_INT: f32 = 0.3;
const RETURN_TO_LINES_INT: f32 = 0.1;
const RETURN_TO_SPEED_LINES_ROLL: f32 = -0.3;
const RETURN_TO_SCENE_GREY: f32 = 0.5;
const RETURN_TO_POSTERIZE: vec3<f32> = vec3<f32>(5.95, 36.42, 4.0);

fn return_to_hash(p: vec2<f32>) -> f32 {
    let q = vec2<u32>(vec2<i32>(floor(p)) + vec2<i32>(32768));
    var h = q.x * 1597334677u ^ q.y * 3812015801u;
    h = h * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h) / 4294967295.0;
}

fn return_to_value(p: vec2<f32>) -> f32 {
    let i = floor(p);
    var f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    let a = return_to_hash(i);
    let b = return_to_hash(i + vec2<f32>(1.0, 0.0));
    let c = return_to_hash(i + vec2<f32>(0.0, 1.0));
    let d = return_to_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Shader Graph's Simple Noise: three octaves of value noise.
fn return_to_noise(uv: vec2<f32>, scale: f32) -> f32 {
    var t = 0.0;
    for (var i = 0; i < 3; i = i + 1) {
        let freq = pow(2.0, f32(i));
        let amp = pow(0.5, f32(3 - i));
        t = t + return_to_value(uv * scale / freq) * amp;
    }
    return t;
}

// Shader Graph's Contrast node.
fn return_to_contrast(x: f32, c: f32) -> f32 {
    let mid = pow(0.5, 2.2);
    return (x - mid) * c + mid;
}

// Stand-in for T_SexyMouse2, in image coordinates (top row first).
fn return_to_mask(uv: vec2<f32>) -> f32 {
    let head = length((uv - vec2<f32>(0.5, 0.6)) / vec2<f32>(0.3, 0.22));
    let left = length(uv - vec2<f32>(0.36, 0.33)) / 0.1;
    let right = length(uv - vec2<f32>(0.7, 0.33)) / 0.12;
    let d = min(head, min(left, right));
    return 1.0 - smoothstep(0.9, 1.0, d);
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    // Unity's v runs up the texture; runity's runs down.
    let v = 1.0 - in.uv.y;
    let r = length(vec2<f32>(in.uv.x, v) - vec2<f32>(0.5));
    let back = vec3<f32>(RETURN_TO_SCENE_GREY) * return_to_contrast(r, -0.48);

    let g = v + v * 2.0 + RETURN_TO_SPEED_LINES_ROLL * in.time;
    let n = return_to_noise(vec2<f32>(g), 150.0);
    let lines = floor(vec3<f32>(n) * RETURN_TO_POSTERIZE) / RETURN_TO_POSTERIZE;

    o.albedo = vec3<f32>(0.0);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = mix(back, lines, RETURN_TO_LINES_INT);
    o.alpha = clamp(RETURN_TO_ALL_INT * return_to_mask(in.uv), 0.0, 1.0);
    return o;
}
