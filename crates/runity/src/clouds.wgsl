// Clouds (clouds.rs): a layer of them marched through at a quarter of the
// frame's resolution; the sky pass lays the result over the sky.
//
// The density is 3D noise — a broad shape eaten into at the edges by a
// finer one — kept to a slab of air between `base` and `base + thickness`,
// rounded at its floor and top, and drifting with the wind. Light comes in
// from the sun through the cloud above each step (Beer's law, and a softer
// share for light scattered many times inside) and from the sky all round.

struct Cloud {
    inverse_view_projection: mat4x4<f32>,
    // eye; w time in seconds
    eye: vec4<f32>,
    // towards the sun; w the sun's intensity
    to_sun: vec4<f32>,
    // the sun's colour after the air; w unused
    sun: vec4<f32>,
    // the sky's light from all round; w unused
    ambient: vec4<f32>,
    // coverage, base (m), thickness (m), density
    shape: vec4<f32>,
    // wind x, wind z (m/s), size of the shapes (m), how far they reach (m)
    drift: vec4<f32>,
    // target width, height
    size: vec4<f32>,
};

@group(0) @binding(0) var<uniform> cloud: Cloud;
@group(0) @binding(1) var clouds_out: texture_storage_2d<rgba16float, write>;

fn hash3(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q += dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

fn noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(hash3(i), hash3(i + vec3<f32>(1.0, 0.0, 0.0)), u.x),
        mix(hash3(i + vec3<f32>(0.0, 1.0, 0.0)), hash3(i + vec3<f32>(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(hash3(i + vec3<f32>(0.0, 0.0, 1.0)), hash3(i + vec3<f32>(1.0, 0.0, 1.0)), u.x),
        mix(hash3(i + vec3<f32>(0.0, 1.0, 1.0)), hash3(i + vec3<f32>(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}

/// How much cloud there is at a point, 0 to about 1.
fn cloud_density(p: vec3<f32>, detail: bool) -> f32 {
    let base = cloud.shape.y;
    let thickness = cloud.shape.z;
    let h = (p.y - base) / thickness;
    if h <= 0.0 || h >= 1.0 {
        return 0.0;
    }
    let drifted = p - vec3<f32>(cloud.drift.x, 0.0, cloud.drift.y) * cloud.eye.w;
    let q = drifted / cloud.drift.z;
    var shape = noise3(q) * 0.55 + noise3(q * 2.03) * 0.28 + noise3(q * 4.1) * 0.17;
    // Rounded floor and top.
    let profile = smoothstep(0.0, 0.2, h) * smoothstep(1.0, 0.55, h);
    let coverage = cloud.shape.x;
    var d = clamp((shape * profile - (1.0 - coverage)) / max(coverage, 0.05), 0.0, 1.0);
    if detail && d > 0.0 {
        let erode = noise3(q * 11.0 + vec3<f32>(0.0, cloud.eye.w * 0.02, 0.0));
        d = clamp(d - erode * 0.35 * (1.0 - d), 0.0, 1.0);
    }
    return d * cloud.shape.w;
}

fn henyey(cos: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (12.566371 * pow(max(1.0 + g2 - 2.0 * g * cos, 1e-4), 1.5));
}

@compute @workgroup_size(8, 8, 1)
fn cs_clouds(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<u32>(cloud.size.xy);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let uv = (vec2<f32>(id.xy) + 0.5) / cloud.size.xy;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let a = cloud.inverse_view_projection * vec4<f32>(ndc, 0.0, 1.0);
    let b = cloud.inverse_view_projection * vec4<f32>(ndc, 1.0, 1.0);
    let d = normalize(b.xyz / b.w - a.xyz / a.w);
    let eye = cloud.eye.xyz;

    // Where the ray is inside the slab.
    let base = cloud.shape.y;
    let top = base + cloud.shape.z;
    var result = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if cloud.shape.x <= 0.0 || abs(d.y) < 1e-4 {
        textureStore(clouds_out, id.xy, result);
        return;
    }
    let t_base = (base - eye.y) / d.y;
    let t_top = (top - eye.y) / d.y;
    let t0 = max(min(t_base, t_top), 0.0);
    let t1 = min(max(t_base, t_top), cloud.drift.w);
    if t1 <= t0 {
        textureStore(clouds_out, id.xy, result);
        return;
    }

    let steps = 28;
    let step = (t1 - t0) / f32(steps);
    let to_sun = cloud.to_sun.xyz;
    let cos = dot(d, to_sun);
    // Forward and back lobes: a silver lining towards the sun.
    let phase = mix(henyey(cos, 0.6), henyey(cos, -0.2), 0.3);
    let jitter = fract(sin(dot(vec2<f32>(id.xy), vec2<f32>(12.9898, 78.233))) * 43758.547);
    var through = 1.0;
    var light = vec3<f32>(0.0);
    for (var i = 0; i < steps; i = i + 1) {
        let t = t0 + step * (f32(i) + jitter);
        let p = eye + d * t;
        let density = cloud_density(p, true);
        if density <= 0.001 {
            continue;
        }
        // Towards the sun: how much cloud the light comes through.
        var above = 0.0;
        for (var j = 1; j <= 4; j = j + 1) {
            above += cloud_density(p + to_sun * (f32(j) * f32(j) * 60.0), false) * f32(2 * j - 1) * 60.0;
        }
        let extinction = density * 0.04;
        // Beer's law towards the sun, and a softer share for the light that
        // has scattered many times inside and gets out anyway.
        let sun_through = exp(-above * 0.04);
        let many = exp(-above * 0.0015) * 0.5;
        let lit = cloud.sun.rgb * cloud.to_sun.w * (sun_through * phase * 10.0 + many)
            + cloud.ambient.rgb * 1.6 * (0.5 + 0.5 * clamp((p.y - base) / cloud.shape.z, 0.0, 1.0));
        let passed = exp(-extinction * step);
        light += through * lit * (1.0 - passed);
        through *= passed;
        if through < 0.01 {
            break;
        }
    }
    // Far clouds fade into the sky.
    let fade = 1.0 - smoothstep(cloud.drift.w * 0.5, cloud.drift.w, t0);
    result = vec4<f32>(light * fade, mix(1.0, through, fade));
    textureStore(clouds_out, id.xy, result);
}
