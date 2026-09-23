// From Assets/Content/Art/Materials/Atlas_Shader 1.shadergraph (URP Lit).
// The atlas colour (UV0) overlaid with a greyscale grunge texture projected
// triplanar at 25%; optionally tops lightened toward white ("Up Vecto?")
// and a faint near-camera pulse in emission ("Blink?").
// Port: the grunge texture (JB_HPBase_FlameRake_BW) is procedural value
// noise; projection uses world position, not object position (Translate
// and Rotation are zero in every material, so only that differs). The
// blink's scene-depth falloff is dropped. The two toggles come from the
// material (M_Atlas_Triplanar_01 1 has Up Vecto?, M_Atlas_Triplanar_Blink_01
// has Blink?); the other values are the graph's defaults, which the three
// materials share.
// runity:params _Up_Vecto _Blink

const ATLAS_TILING: f32 = 0.31;
const ATLAS_BLEND: f32 = 1.0;
const ATLAS_TRIPLANAR_INT: f32 = 0.25;
const ATLAS_SMOOTHNESS: f32 = 0.426; // _Roughtnes, wired to Smoothness

fn atlas_hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(123.34, 456.21));
    let r = q + dot(q, q + 45.32);
    return fract(r.x * r.y);
}

fn atlas_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = atlas_hash(i);
    let b = atlas_hash(i + vec2<f32>(1.0, 0.0));
    let c = atlas_hash(i + vec2<f32>(0.0, 1.0));
    let d = atlas_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Stand-in for the grunge texture: sharp-edged grey patches, sRGB mean
// about 0.4 like the original, returned linear.
fn atlas_grunge(uv: vec2<f32>) -> f32 {
    let p = uv * 6.0;
    let n = atlas_noise(p) * 0.6 + atlas_noise(p * 2.3 + 7.1) * 0.3 + atlas_noise(p * 5.1 + 3.7) * 0.1;
    let patches = floor(n * 5.0) / 4.0;
    let srgb = clamp(0.15 + patches * 0.5, 0.0, 1.0);
    return pow(srgb, 2.2);
}

fn atlas_overlay(base: vec3<f32>, blend: vec3<f32>) -> vec3<f32> {
    let low = 2.0 * base * blend;
    let high = 1.0 - 2.0 * (1.0 - base) * (1.0 - blend);
    return select(high, low, base <= vec3<f32>(0.5));
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let p = in.world_position * ATLAS_TILING;
    var w = pow(abs(in.normal), vec3<f32>(ATLAS_BLEND));
    w = w / max(w.x + w.y + w.z, 1e-5);
    let tri = atlas_grunge(p.zy) * w.x + atlas_grunge(p.xz) * w.y + atlas_grunge(p.xy) * w.z;
    var color = mix(out.albedo, atlas_overlay(out.albedo, vec3<f32>(tri)), ATLAS_TRIPLANAR_INT);
    if in.params[0].x > 0.5 {
        // The up texture is unset in the material, so it samples white.
        let mid = pow(0.5, 2.2);
        let up = clamp(dot(in.normal, vec3<f32>(0.0, 1.0, 0.0)), 0.0, 1.0);
        let t = ((up - mid) * 2.73 + mid) * 0.2;
        color = mix(color, vec3<f32>(1.0), t);
    }
    o.albedo = color;
    o.metallic = 0.0;
    o.smoothness = ATLAS_SMOOTHNESS;
    if in.params[0].y > 0.5 {
        o.emission = vec3<f32>(clamp(sin(in.time / 3.0 * 6.0) / 30.0, 0.0, 1.0));
    } else {
        o.emission = vec3<f32>(0.0);
    }
    return o;
}
