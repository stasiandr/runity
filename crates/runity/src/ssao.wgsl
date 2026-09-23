// Screen-space ambient occlusion: how much of the sky a point can see,
// guessed from the depth around it. URP's SSAO renderer feature.
//
// From the depth and normals of a prepass: for each pixel, a handful of
// points in the hemisphere over its surface, each checked against the
// depth buffer — one that is behind something the camera sees is in a
// crevice. The share of those, raised to the intensity, darkens what light
// comes from all around; then a blur hides the noise the random rotation
// traded the banding for.

struct Ssao {
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    // eye position; w unused
    eye: vec4<f32>,
    // radius (metres), intensity, falloff distance, sample count
    params: vec4<f32>,
    // width, height, 1/width, 1/height
    size: vec4<f32>,
    kernel: array<vec4<f32>, 16>,
};

@group(0) @binding(0) var<uniform> ssao: Ssao;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var normals: texture_2d<f32>;
@group(0) @binding(3) var source: texture_2d<f32>;

struct Varyings {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> Varyings {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var out: Varyings;
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

fn world_at(pixel: vec2<i32>) -> vec3<f32> {
    let d = textureLoad(depth, pixel, 0);
    let uv = (vec2<f32>(pixel) + 0.5) * ssao.size.zw;
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let world = ssao.inverse_view_projection * ndc;
    return world.xyz / world.w;
}

fn hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(443.897, 441.423));
    let r = q + dot(q, q.yx + 19.19);
    return fract((r.x + r.y) * r.x);
}

@fragment
fn fs_occlusion(in: Varyings) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    if textureLoad(depth, pixel, 0) >= 1.0 {
        // The sky: nothing to occlude.
        return vec4<f32>(1.0);
    }
    let p = world_at(pixel);
    let n = normalize(textureLoad(normals, pixel, 0).xyz);
    let distance = length(p - ssao.eye.xyz);
    let radius = ssao.params.x;

    // A turn about the normal from a 4x4 pattern: sixteen turns, one per
    // pixel of each 4x4 block, which the 4x4 blur then averages away
    // exactly — rather than the same turn everywhere, which bands, or
    // random ones, which leave grain.
    let cell = vec2<u32>(pixel) % vec2<u32>(4u);
    let order = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    let angle = (order[cell.y * 4u + cell.x] + 0.5) / 16.0 * 6.2831853;
    var r = vec3<f32>(cos(angle), sin(angle), 0.37);
    if abs(dot(normalize(r), n)) > 0.95 {
        r = vec3<f32>(0.37, cos(angle), sin(angle));
    }
    let t = normalize(r - n * dot(r, n));
    let b = cross(n, t);

    let count = u32(ssao.params.w);
    var occluded = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let k = ssao.kernel[i].xyz;
        let at = p + (t * k.x + b * k.y + n * k.z) * radius;
        let clip = ssao.view_projection * vec4<f32>(at, 1.0);
        if clip.w <= 0.0 {
            continue;
        }
        let ndc = clip.xyz / clip.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
            continue;
        }
        let there = world_at(vec2<i32>(uv * ssao.size.xy));
        let seen = length(there - ssao.eye.xyz);
        let sample_distance = length(at - ssao.eye.xyz);
        // In front of the sample: it is inside something — past a margin
        // that grows with distance, so a flat floor's own pixels do not
        // shadow it. And only if that something is near this point: what
        // stands a metre in front of a wall does not darken the wall.
        let margin = max(radius * 0.05, sample_distance * 0.003);
        if seen < sample_distance - margin {
            let gap = abs(distance - seen);
            occluded += 1.0 - smoothstep(radius * 0.5, radius, gap);
        }
    }
    var ao = 1.0 - occluded / max(f32(count), 1.0);
    ao = pow(clamp(ao, 0.0, 1.0), ssao.params.y);
    // Fades out with distance, as URP's Falloff Distance.
    let falloff = ssao.params.z;
    let fade = clamp((falloff - distance) / (falloff * 0.2), 0.0, 1.0);
    return vec4<f32>(mix(1.0, ao, fade));
}

// A 4x4 box — the pattern's sixteen turns averaged away — over what lies
// at about the same depth only, so the occlusion of the ground does not
// spill onto the edge of what stands on it.
@fragment
fn fs_blur(in: Varyings) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let limit = vec2<i32>(ssao.size.xy) - vec2<i32>(1);
    let here = length(world_at(pixel) - ssao.eye.xyz);
    var sum = 0.0;
    var weight = 0.0;
    for (var y = -2; y < 2; y = y + 1) {
        for (var x = -2; x < 2; x = x + 1) {
            let at = clamp(pixel + vec2<i32>(x, y), vec2<i32>(0), limit);
            let there = length(world_at(at) - ssao.eye.xyz);
            let w = 1.0 - smoothstep(0.02, 0.1, abs(there - here) / max(here, 1e-3));
            sum += textureLoad(source, at, 0).r * w;
            weight += w;
        }
    }
    return vec4<f32>(sum / max(weight, 1e-4));
}
