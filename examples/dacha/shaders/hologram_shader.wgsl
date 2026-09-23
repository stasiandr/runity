// From Assets/Content/Art/Materials/Hologram_Shader.shadergraph (URP Unlit,
// transparent, both faces). Values from M_Hologram_01.mat.
//
// The original glows in _Color by a Fresnel term and fades its alpha with a
// vertical gradient, minus fine scrolling noise lines, plus bright scrolling
// bands sampled from T_SunLines_M, all flickering with sin(2t) and noise.
//
// This port: the T_SunLines_M bands are procedural stripes; the optional
// _Mask / _Blink texture branch (off in M_Hologram_01) is left out; the
// colour goes to emission over a black albedo since runity lights the
// surface. The M_Hologram_01 .mat says _Surface: 0, so unless the material
// is made transparent the alpha has no effect.

const HOLOGRAM_COLOR: vec3<f32> = vec3<f32>(0.0, 0.8563, 0.1584);
const HOLOGRAM_FRESNEL_POWER: f32 = 0.35;
const HOLOGRAM_SPEED_SMALL: f32 = 0.21;
const HOLOGRAM_SPEED_BIG: f32 = 0.25;
const HOLOGRAM_INT_SMALL: f32 = 0.14;
const HOLOGRAM_INT_BIG: f32 = 3.0;

fn hologram_hash(p: vec2<f32>) -> f32 {
    let q = vec2<u32>(vec2<i32>(floor(p)) + vec2<i32>(32768));
    var h = q.x * 1597334677u ^ q.y * 3812015801u;
    h = h * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h) / 4294967295.0;
}

fn hologram_value(p: vec2<f32>) -> f32 {
    let i = floor(p);
    var f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    let a = hologram_hash(i);
    let b = hologram_hash(i + vec2<f32>(1.0, 0.0));
    let c = hologram_hash(i + vec2<f32>(0.0, 1.0));
    let d = hologram_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Shader Graph's Simple Noise: three octaves of value noise.
fn hologram_noise(uv: vec2<f32>, scale: f32) -> f32 {
    var t = 0.0;
    for (var i = 0; i < 3; i = i + 1) {
        let freq = pow(2.0, f32(i));
        let amp = pow(0.5, f32(3 - i));
        t = t + hologram_value(uv * scale / freq) * amp;
    }
    return t;
}

// Stand-in for T_SunLines_M sampled at (g, g): soft horizontal streaks of
// uneven brightness, dark towards the texture's left and right edges.
fn hologram_sunlines(g: f32) -> f32 {
    let y = g * 14.0;
    let row = floor(y);
    let f = fract(y);
    let width = mix(0.08, 0.35, hologram_hash(vec2<f32>(row, 7.0)));
    let d = (f - 0.5) / width;
    let bright = 0.6 * pow(hologram_hash(vec2<f32>(row, 3.0)), 1.5);
    let x = fract(g);
    let edges = smoothstep(0.08, 0.3, x) * (1.0 - smoothstep(0.7, 0.9, x));
    return bright * exp(-d * d) * edges;
}

// Unity's gradient: keys 0 -> 0, 0.038 -> 0.03, 0.488 -> 1, 0.976 -> 0.
fn hologram_gradient(t: f32) -> f32 {
    if t < 0.0382 {
        return mix(0.0, 0.0301, clamp(t / 0.0382, 0.0, 1.0));
    }
    if t < 0.4882 {
        return mix(0.0301, 1.0, (t - 0.0382) / 0.45);
    }
    return mix(1.0, 0.0, clamp((t - 0.4882) / 0.4883, 0.0, 1.0));
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let t = in.time;
    // Unity's v runs up the texture; runity's runs down.
    let v = 1.0 - in.uv.y;
    let view = normalize(frame.camera_position.xyz - in.world_position);
    let fresnel = pow(1.0 - clamp(dot(normalize(in.normal), view), 0.0, 1.0), HOLOGRAM_FRESNEL_POWER);

    let small_g = v + HOLOGRAM_SPEED_SMALL * t;
    let small = hologram_noise(vec2<f32>(small_g), 500.0) * HOLOGRAM_INT_SMALL;
    let big_g = v + HOLOGRAM_SPEED_BIG * t;
    let big = hologram_sunlines(big_g) * HOLOGRAM_INT_BIG;
    let lines = clamp(hologram_gradient(v) * fresnel - (small - big), 0.0, 1.0);

    let pulse = clamp(sin(t * 2.0), 0.0, 1.0) + 0.8;
    let flicker = mix(pulse, pulse * hologram_noise(vec2<f32>(t), 53.1), 0.5);

    o.albedo = vec3<f32>(0.0);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = HOLOGRAM_COLOR * fresnel;
    o.alpha = lines * flicker;
    return o;
}
