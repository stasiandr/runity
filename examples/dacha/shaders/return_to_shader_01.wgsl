// From Assets/Content/Art/Materials/Return_To_Shader_01.shadergraph (URP
// Unlit, transparent, both faces). Values from the M_Return.mat in
// _Incoming/Tools/SexyMouseTrap (the one the prefab uses); _ALL_INT,
// _Lines_Int and _Speed_Lines_Roll come from the material (the M_Return.mat
// in Art/Materials has _ALL_INT 0 and draws nothing).
// scrap:params _ALL_INT _Lines_Int _Speed_Lines_Roll
// scrap:textures _Mask
//
// A "rewind" overlay: the scene behind, read with a noise-jittered screen
// position and made grey, darkened towards the edges, with faint posterized
// noise lines rolling over it; alpha is _ALL_INT x a mask texture
// (T_SexyMouse2, a mouse-head silhouette).
//
// This port: the scene colour cannot be read, so a flat grey stands in for
// it and the jitter is dropped. The mask's red channel is read from the
// material's _Mask (white where it has none, as in Art/Materials'
// M_Return).

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

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    // Unity's v runs up the texture; scrap's runs down.
    let v = 1.0 - in.uv.y;
    let r = length(vec2<f32>(in.uv.x, v) - vec2<f32>(0.5));
    let back = vec3<f32>(RETURN_TO_SCENE_GREY) * return_to_contrast(r, -0.48);

    let g = v + v * 2.0 + in.params[0].z * in.time;
    let n = return_to_noise(vec2<f32>(g), 150.0);
    let lines = floor(vec3<f32>(n) * RETURN_TO_POSTERIZE) / RETURN_TO_POSTERIZE;

    o.albedo = vec3<f32>(0.0);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = mix(back, lines, in.params[0].y);
    o.alpha = clamp(in.params[0].x * texture_at(in, 0u, in.uv).r, 0.0, 1.0);
    return o;
}
