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
    previous_view_projection: mat4x4<f32>,
    // how much light bounces (0: none), how far its rays reach; 1 for GTAO
    bounce: vec4<f32>,
};

@group(0) @binding(0) var<uniform> ssao: Ssao;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var normals: texture_2d<f32>;
@group(0) @binding(3) var source: texture_2d<f32>;
// The last frame, lit: what a bounce ray finds is lit as it was then.
@group(0) @binding(4) var last_frame: texture_2d<f32>;

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

/// Light bounced onto `p` from what is near: rays over the hemisphere of
/// `n` (turned by `t`, `b` as the occlusion's are), stepped across the
/// screen; one that passes just behind what the camera sees has met it,
/// and brings back its colour last frame — if it faced the ray. Averaged
/// over all the rays, a miss bringing nothing: the sky and the ground
/// are the hemisphere ambient's, which the occlusion already shades.
fn bounced(p: vec3<f32>, n: vec3<f32>, t: vec3<f32>, b: vec3<f32>, jitter: f32) -> vec3<f32> {
    let radius = ssao.bounce.y;
    let rays = 4u;
    let steps = 10u;
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < rays; i = i + 1u) {
        // Cosine-weighted: a ray's share is already its cosine.
        let u = (f32(i) + 0.5) / f32(rays);
        let phi = f32(i) * 2.3999632;
        let across = sqrt(u);
        let dir = normalize(t * cos(phi) * across + b * sin(phi) * across + n * sqrt(1.0 - u));
        let start = p + n * 0.03;
        var previous = 0.0;
        for (var k = 1u; k <= steps; k = k + 1u) {
            // Where the steps fall moves with the pattern too, so the blur
            // averages their bands away.
            let s = (f32(k) - jitter) / f32(steps);
            let along = max(radius * s * s, 0.05);
            let q = start + dir * along;
            let clip = ssao.view_projection * vec4<f32>(q, 1.0);
            if clip.w <= 0.0 {
                break;
            }
            let ndc = clip.xyz / clip.w;
            let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
            if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
                break;
            }
            let at = vec2<i32>(uv * ssao.size.xy);
            if textureLoad(depth, at, 0) >= 1.0 {
                continue;
            }
            if behind_at(q, at) > 0.0 {
                // Past something: found closer by halving back towards the
                // step before, so where it lands does not jump from step to
                // step in bands. Thin enough there to be what the ray met,
                // and facing it: its light, where it was last frame.
                var lo = previous;
                var hi = along;
                for (var h = 0; h < 4; h = h + 1) {
                    let mid = (lo + hi) * 0.5;
                    let m = start + dir * mid;
                    if behind_at(m, pixel_of(m)) > 0.0 {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                let hit = start + dir * hi;
                let at = pixel_of(hit);
                let there = world_at(at);
                let facing = -dot(normalize(textureLoad(normals, at, 0).xyz), dir);
                let was = ssao.previous_view_projection * vec4<f32>(there, 1.0);
                let then = was.xy / was.w;
                let back = vec2<f32>(then.x * 0.5 + 0.5, 0.5 - then.y * 0.5);
                let thin = behind_at(hit, at) < 0.3 + hi * 0.25;
                if thin && facing > 0.0 && was.w > 0.0 && all(back >= vec2<f32>(0.0)) && all(back <= vec2<f32>(1.0)) {
                    let seen = textureLoad(last_frame, vec2<i32>(back * ssao.size.xy), 0).rgb;
                    // Less of it further off, and nothing at the end of the
                    // reach — a hard end there draws a line along every
                    // place a ray just reaches something.
                    let reach = hi / radius;
                    sum += min(seen, vec3<f32>(8.0)) * (1.0 - reach * reach);
                }
                break;
            }
            previous = along;
        }
    }
    return sum / f32(rays) * ssao.bounce.x;
}

/// The pixel `q` falls on, kept on the screen.
fn pixel_of(q: vec3<f32>) -> vec2<i32> {
    let c = ssao.view_projection * vec4<f32>(q, 1.0);
    let n = c.xy / c.w;
    return clamp(
        vec2<i32>(vec2<f32>(n.x * 0.5 + 0.5, 0.5 - n.y * 0.5) * ssao.size.xy),
        vec2<i32>(0),
        vec2<i32>(ssao.size.xy) - vec2<i32>(1),
    );
}

/// How far `q` is behind what the camera sees at `pixel`, past a margin
/// that grows with distance; 0 or less when it is in front.
fn behind_at(q: vec3<f32>, pixel: vec2<i32>) -> f32 {
    let far = length(q - ssao.eye.xyz);
    if textureLoad(depth, pixel, 0) >= 1.0 {
        return -1.0;
    }
    let seen = length(world_at(pixel) - ssao.eye.xyz);
    return far - seen - (0.02 + far * 0.003);
}

@fragment
fn fs_occlusion(in: Varyings) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    if textureLoad(depth, pixel, 0) >= 1.0 {
        // The sky: nothing to occlude, nothing to bounce.
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
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
    let turn = (order[cell.y * 4u + cell.x] + 0.5) / 16.0;
    let angle = turn * 6.2831853;
    var r = vec3<f32>(cos(angle), sin(angle), 0.37);
    if abs(dot(normalize(r), n)) > 0.95 {
        r = vec3<f32>(0.37, cos(angle), sin(angle));
    }
    let t = normalize(r - n * dot(r, n));
    let b = cross(n, t);

    var ao = 1.0;
    if ssao.bounce.z > 0.5 {
        ao = gtao(pixel, p, n, distance, turn);
    } else {
        ao = hemisphere_occlusion(p, n, t, b, distance);
    }
    ao = pow(clamp(ao, 0.0, 1.0), ssao.params.y);
    // Fades out with distance, as URP's Falloff Distance.
    let falloff = ssao.params.z;
    let fade = clamp((falloff - distance) / (falloff * 0.2), 0.0, 1.0);
    var light = vec3<f32>(0.0);
    if ssao.bounce.x > 0.0 {
        light = bounced(p, n, t, b, fract(turn * 5.0)) * fade;
    }
    return vec4<f32>(light, mix(1.0, ao, fade));
}

/// URP's SSAO: the share of points in the hemisphere over `n` that fall
/// behind what the camera sees, near enough to be what occludes.
fn hemisphere_occlusion(p: vec3<f32>, n: vec3<f32>, t: vec3<f32>, b: vec3<f32>, distance: f32) -> f32 {
    let radius = ssao.params.x;
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
    return 1.0 - occluded / max(f32(count), 1.0);
}

/// The pixel a world point falls on, unclamped, as floats.
fn screen_of(q: vec3<f32>) -> vec3<f32> {
    let c = ssao.view_projection * vec4<f32>(q, 1.0);
    let n = c.xy / c.w;
    return vec3<f32>(vec2<f32>(n.x * 0.5 + 0.5, 0.5 - n.y * 0.5) * ssao.size.xy, c.w);
}

/// Ground-truth ambient occlusion (Jimenez, Wu, Pesce, Jarabo 2016): in two
/// slices through the view at `p` — turned by `turn`, which the blur's
/// sixteen turns fill out — the highest the depth rises within the radius
/// on either side, the horizons; then the cosine-weighted share of the dome
/// between them over the normal, as the integral in closed form.
fn gtao(pixel: vec2<i32>, p: vec3<f32>, n: vec3<f32>, distance: f32, turn: f32) -> f32 {
    let radius = ssao.params.x;
    let v = normalize(ssao.eye.xyz - p);
    // The radius on the screen: what a metre across the view is there.
    var across = cross(v, vec3<f32>(0.0, 1.0, 0.0));
    if dot(across, across) < 1e-4 {
        across = cross(v, vec3<f32>(1.0, 0.0, 0.0));
    }
    across = normalize(across);
    let here = screen_of(p);
    let reach = length(screen_of(p + across * radius).xy - here.xy);
    if reach < 1.0 {
        return 1.0;
    }
    let slices = 2u;
    let steps = 8u;
    let limit = vec2<f32>(ssao.size.xy) - 1.0;
    var visible = 0.0;
    for (var s = 0u; s < slices; s = s + 1u) {
        let phi = (f32(s) + turn) / f32(slices) * 3.14159265;
        let dir = vec2<f32>(cos(phi), sin(phi));
        // Which way the slice runs in the world: the point a pixel along it
        // on the screen, at the same depth.
        let clip = ssao.view_projection * vec4<f32>(p, 1.0);
        let ndc = clip.xyz / clip.w;
        let step_ndc = vec2<f32>(dir.x, -dir.y) * 2.0 * ssao.size.zw;
        let beside = ssao.inverse_view_projection * vec4<f32>(ndc.xy + step_ndc, ndc.z, 1.0);
        var way3 = beside.xyz / beside.w - p;
        if dot(way3, way3) < 1e-12 {
            continue;
        }
        way3 = normalize(way3 - v * dot(way3, v));
        // Horizons on the two sides, as cosines against the view.
        var high = array<f32, 2>(-1.0, -1.0);
        for (var side = 0u; side < 2u; side = side + 1u) {
            let way = select(-dir, dir, side == 0u);
            for (var k = 1u; k <= steps; k = k + 1u) {
                let t = (f32(k) - 0.5 + fract(turn * 7.0 + f32(k) * 0.37) * 0.5) / f32(steps);
                let at = clamp(here.xy + way * reach * t * t, vec2<f32>(0.0), limit);
                let texel = vec2<i32>(at);
                if textureLoad(depth, texel, 0) >= 1.0 {
                    continue;
                }
                let q = world_at(texel);
                let d = q - p;
                let len = length(d);
                if len < 1e-4 {
                    continue;
                }
                // Past the radius it counts less, so a far wall is not a
                // horizon: the cosine lowered toward none.
                let cos_h = dot(d / len, v);
                let fall = clamp(1.0 - len * len / (radius * radius), 0.0, 1.0);
                high[side] = max(high[side], mix(-1.0, cos_h, fall));
            }
        }
        let axis = normalize(cross(way3, v));
        let flat_n = n - axis * dot(n, axis);
        let weight = length(flat_n);
        if weight < 1e-4 {
            continue;
        }
        let pn = flat_n / weight;
        let gamma = sign(dot(way3, pn)) * acos(clamp(dot(pn, v), -1.0, 1.0));
        // The horizon angles, each side, clamped to the hemisphere of the
        // normal.
        let h1 = gamma + min(acos(clamp(high[0], -1.0, 1.0)) - gamma, 1.5707963);
        let h0 = gamma + max(-acos(clamp(high[1], -1.0, 1.0)) - gamma, -1.5707963);
        let arc = -cos(2.0 * h0 - gamma) + cos(gamma) + 2.0 * h0 * sin(gamma)
            - cos(2.0 * h1 - gamma) + cos(gamma) + 2.0 * h1 * sin(gamma);
        visible += weight * 0.25 * arc;
    }
    return visible / f32(slices);
}

// A 4x4 box — the pattern's sixteen turns averaged away — over what lies
// at about the same depth only, so the occlusion of the ground does not
// spill onto the edge of what stands on it.
@fragment
fn fs_blur(in: Varyings) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let limit = vec2<i32>(ssao.size.xy) - vec2<i32>(1);
    let here = length(world_at(pixel) - ssao.eye.xyz);
    var sum = vec4<f32>(0.0);
    var weight = 0.0;
    for (var y = -2; y < 2; y = y + 1) {
        for (var x = -2; x < 2; x = x + 1) {
            let at = clamp(pixel + vec2<i32>(x, y), vec2<i32>(0), limit);
            let there = length(world_at(at) - ssao.eye.xyz);
            let w = 1.0 - smoothstep(0.02, 0.1, abs(there - here) / max(here, 1e-3));
            sum += textureLoad(source, at, 0) * w;
            weight += w;
        }
    }
    return sum / max(weight, 1e-4);
}
