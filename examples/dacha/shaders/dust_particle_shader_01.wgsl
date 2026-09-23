// From Assets/Content/Art/Materials/Dust_Particle_Shader_01.shadergraph (URP Lit).
// A lit dust puff: flat Color for albedo, alpha from the red channel of a
// cloud-shaped mask (T_Dust_01_M), cut out at 0.5.
// Port: the mask is a procedural lumpy blob with noise holes. The cut-out
// is a discard here, since the importer does not turn alpha clip on for
// this material; runity's fade over life stays in alpha. The shadow and AO
// prepasses do not run this function, so there the quad is whole.
// Colour from M_Particle_Dust_01.mat.

const DUST_COLOR: vec3<f32> = vec3<f32>(0.3456, 0.2736, 0.2329); // sRGB 0.623, 0.560, 0.520
const DUST_CLIP: f32 = 0.5;

fn dust_hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(127.1, 311.7));
    let r = q + dot(q, q + 34.57);
    return fract(r.x * r.y);
}

fn dust_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = dust_hash(i);
    let b = dust_hash(i + vec2<f32>(1.0, 0.0));
    let c = dust_hash(i + vec2<f32>(0.0, 1.0));
    let d = dust_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn dust_mask(uv: vec2<f32>) -> f32 {
    let c = (uv - vec2<f32>(0.5, 0.52)) * 2.0;
    let r = length(c);
    let dir = c / max(r, 1e-4);
    // A ragged outline: the radius wanders with direction.
    let edge = 0.45 + 0.3 * dust_noise(dir * 2.2 + 5.0) + 0.1 * dust_noise(dir * 6.0 + 1.3);
    var m = 1.0 - smoothstep(edge * 0.3, edge, r);
    // Bites taken out of the inside.
    m = m * (0.55 + 0.7 * dust_noise(uv * 7.0 + 2.0));
    return clamp(m * 1.3, 0.0, 1.0);
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    if dust_mask(in.uv) < DUST_CLIP {
        discard;
    }
    var o = out;
    o.albedo = DUST_COLOR * out.albedo;
    o.metallic = 0.0;
    o.smoothness = 0.5;
    return o;
}
