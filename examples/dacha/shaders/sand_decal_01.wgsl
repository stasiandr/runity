// Sand decal: a patch of loose sand or grass laid over the ground.
// From Assets/Content/Art/Materials/Sand_Decal_01.shadergraph (M_Sand_Decal_01,
// M_Green_Decal_01). A URP decal: colour from _Texture2D (T_Decal_Sand_01 or
// T_Grass_01_D) at UV x1.55, alpha from the T_Sand_Decal_01_M mask's red
// times _Mask_int (1).
// Here it is a surface on whatever mesh carries it, not a projector. The
// colour is the material's own base colour and base map (give it the
// _Texture2D picture, tiled 1.55), with a faint procedural grain; the mask, a
// soft cluster of blobs, is procedural on the UVs. Alpha only shows if the
// material is transparent.

const SAND_DECAL_MASK_INT = 1.0;

fn sand_decal_hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(123.34, 456.21));
    let r = q + dot(q, q + 45.32);
    return fract(r.x * r.y);
}

fn sand_decal_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = sand_decal_hash(i);
    let b = sand_decal_hash(i + vec2<f32>(1.0, 0.0));
    let c = sand_decal_hash(i + vec2<f32>(0.0, 1.0));
    let d = sand_decal_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// A few soft round blobs clustered about the middle, as in the mask texture.
fn sand_decal_mask(uv: vec2<f32>) -> f32 {
    var m = 0.0;
    for (var i = 0; i < 7; i = i + 1) {
        let s = f32(i) * 7.31;
        let centre = vec2<f32>(0.5) + (vec2<f32>(sand_decal_hash(vec2<f32>(s, 1.0)), sand_decal_hash(vec2<f32>(s, 2.0))) - 0.5) * 0.5;
        let radius = mix(0.08, 0.16, sand_decal_hash(vec2<f32>(s, 3.0)));
        let d = length(uv - centre) / radius;
        m = max(m, exp(-d * d * 1.5));
    }
    let edge = 1.0 - smoothstep(0.35, 0.5, length(uv - 0.5));
    let wobble = mix(0.8, 1.1, sand_decal_noise(uv * 9.0));
    return clamp(m * edge * wobble, 0.0, 1.0);
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let grain = mix(0.94, 1.06, sand_decal_noise(in.uv * 1.55 * 60.0));
    o.albedo = out.albedo * grain;
    o.alpha = out.alpha * sand_decal_mask(in.uv) * SAND_DECAL_MASK_INT;
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = vec3<f32>(0.0);
    return o;
}
