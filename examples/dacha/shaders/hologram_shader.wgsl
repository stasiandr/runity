// From Assets/Content/Art/Materials/Hologram_Shader.shadergraph (URP Unlit,
// transparent, both faces). _Color (sRGB), the Fresnel power and the two
// line sets' strengths and speeds come from the material.
// scrap:params _Color.r _Color.g _Color.b _Fresnel_int _Int_Small_Lines _Int_Big_Lines _Speed_Small_Lines _Speed_Big_Lines
// scrap:textures _SampleTexture2D_d1e16623bfa34629bae7ce91e3854ab1_Texture_1_Texture2D
//
// The original glows in _Color by a Fresnel term and fades its alpha with a
// vertical gradient, minus fine scrolling noise lines, plus bright scrolling
// bands sampled from T_SunLines_M, all flickering with sin(2t) and noise.
//
// This port reads T_SunLines_M from the material. The _Mask / _Blink
// branch (on only in M_Eyes: alpha plus its _Mask_1 / _Mask_2 textures,
// blinking between them) is left out: the switch is a ninth number the
// params line has no room for. The colour goes to emission over a black
// albedo.

fn hologram_srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

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
    // Unity's v runs up the texture; scrap's runs down.
    let v = 1.0 - in.uv.y;
    let view = normalize(frame.camera_position.xyz - in.world_position);
    let fresnel = pow(1.0 - clamp(dot(normalize(in.normal), view), 0.0, 1.0), in.params[0].w);

    let small_g = v + in.params[1].z * t;
    let small = hologram_noise(vec2<f32>(small_g), 500.0) * in.params[1].x;
    let big_g = v + in.params[1].w * t;
    // Unity samples at (g, g); its v runs up the texture, scrap's down.
    let big = texture_at(in, 0u, vec2<f32>(big_g, 1.0 - big_g)).r * in.params[1].y;
    let lines = clamp(hologram_gradient(v) * fresnel - (small - big), 0.0, 1.0);

    let pulse = clamp(sin(t * 2.0), 0.0, 1.0) + 0.8;
    let flicker = mix(pulse, pulse * hologram_noise(vec2<f32>(t), 53.1), 0.5);

    o.albedo = vec3<f32>(0.0);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = hologram_srgb_to_linear(in.params[0].xyz) * fresnel;
    o.alpha = lines * flicker;
    return o;
}
