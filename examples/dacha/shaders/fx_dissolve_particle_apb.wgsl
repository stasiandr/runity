// From Assets/Thirdparty/VFX_Klaus/Shaders/Fx_dissolve_particle_apb.shadergraph (URP Unlit, transparent).
// A splash sprite that dissolves over its life: the texture's G channel,
// pushed by particle custom data (progress, edge sharpness, UV
// distortion by the B channel), eats the A-channel shape away; colour is
// vertex colour times R, lerped toward a custom-data highlight colour by a
// remapped band of the same dissolve value; depth-soft edges.
// Port: the sprite sheet (Tex_fx_splash_set / Tex_fx_elements_set_01) is a
// procedural ragged splash. There are no custom data streams, so life is
// read from runity's fade as 1 - alpha, and custom data follows the curves
// in Booooom.prefab (progress = life, sharpness = 20 * life, distortion =
// -0.1 * life). The highlight colour is white, but with the inner
// materials' range (-10..-2) it never shows. Soft particles are dropped (no
// scene depth). Unlit is imitated by black albedo and colour as emission.

const FXD_HIGHLIGHT_MIN: f32 = -10.0;
const FXD_HIGHLIGHT_MAX: f32 = -2.0;
const FXD_EMISSION_POWER: f32 = 1.0;
const FXD_HIGHLIGHT: vec4<f32> = vec4<f32>(1.0, 1.0, 1.0, 1.0); // custom data 2: rgb, strength

fn fxd_hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(127.1, 311.7));
    let r = q + dot(q, q + 34.57);
    return fract(r.x * r.y);
}

fn fxd_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = fxd_hash(i);
    let b = fxd_hash(i + vec2<f32>(1.0, 0.0));
    let c = fxd_hash(i + vec2<f32>(0.0, 1.0));
    let d = fxd_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fxd_fbm(p: vec2<f32>) -> f32 {
    return fxd_noise(p) * 0.55 + fxd_noise(p * 2.1 + 3.1) * 0.3 + fxd_noise(p * 4.3 + 7.7) * 0.15;
}

// The sprite: r brightness, g dissolve value, b distortion, a shape.
fn fxd_texture(uv: vec2<f32>) -> vec4<f32> {
    let c = (uv - 0.5) * 2.0;
    let r = length(c);
    let dir = c / max(r, 1e-4);
    let edge = 0.55 + 0.35 * fxd_noise(dir * 2.5 + 4.0) + 0.1 * fxd_noise(dir * 7.0 + 9.0);
    let shape = 1.0 - smoothstep(edge * 0.85, edge, r);
    let g = clamp(fxd_fbm(uv * 5.0) * 0.6 + (1.0 - r) * 0.5, 0.0, 1.0);
    let bright = 0.8 + 0.2 * fxd_noise(uv * 9.0 + 1.0);
    return vec4<f32>(bright, g, fxd_noise(uv * 4.0 + 11.0), shape);
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let life = clamp(1.0 - out.alpha, 0.0, 1.0);
    let progress = life;
    let sharpness = 20.0 * life;
    let distortion = -0.1 * life;

    let warp = fxd_texture(in.uv).b * distortion;
    let t = fxd_texture(vec2<f32>(in.uv.x + warp, in.uv.y));

    let dissolve = t.g + 1.0 - progress;
    let out_min = -(progress * sharpness);
    let mask = clamp(out_min + dissolve * (1.0 - out_min) / (1.0 + 0.1 * progress), 0.0, 1.0);

    let band = clamp(FXD_HIGHLIGHT_MIN + dissolve * (FXD_HIGHLIGHT_MAX - FXD_HIGHLIGHT_MIN), 0.0, 1.0);
    let color = mix(out.albedo * t.r * mask, FXD_HIGHLIGHT.rgb * FXD_EMISSION_POWER, out.alpha * FXD_HIGHLIGHT.a * band);

    o.albedo = vec3<f32>(0.0);
    o.emission = color;
    o.alpha = t.a * mask * out.alpha;
    o.metallic = 0.0;
    o.smoothness = 0.0;
    return o;
}
