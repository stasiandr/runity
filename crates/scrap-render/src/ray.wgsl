// The traced half of the lit shader (ray.rs): what a point sees, asked of
// the scene itself through the hardware's ray queries.

@group(0) @binding(5) var scene_rays: acceleration_structure;

/// 1.0 when nothing among `mask` (RAY_THINGS, RAY_TERRAIN) lies between
/// `start` and `reach` along `direction` from `origin`, 0.0 when something
/// does. Everything in the scene is opaque to rays.
fn ray_clear(origin: vec3<f32>, direction: vec3<f32>, start: f32, reach: f32, mask: u32) -> f32 {
    var query: ray_query;
    rayQueryInitialize(
        &query,
        scene_rays,
        RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT, mask, start, reach, origin, direction),
    );
    rayQueryProceed(&query);
    let hit = rayQueryGetCommittedIntersection(&query);
    return select(1.0, 0.0, hit.kind != RAY_QUERY_INTERSECTION_NONE);
}

/// What a reflection ray reads of the thing it hits (ray.rs's RayMaterial).
struct RayMaterial {
    // linear colour; metallic, or -1 for what is unlit
    color_metal: vec4<f32>,
    // emission; smoothness
    emission_smooth: vec4<f32>,
};

@group(0) @binding(24) var<storage, read> ray_materials: array<RayMaterial>;

struct RayHit {
    t: f32,
    instance: u32,
};

/// The nearest thing among `mask` along a ray: how far, and which. −1
/// when nothing is.
fn ray_nearest(origin: vec3<f32>, direction: vec3<f32>, reach: f32, mask: u32) -> RayHit {
    var query: ray_query;
    rayQueryInitialize(
        &query,
        scene_rays,
        RayDesc(RAY_FLAG_NONE, mask, 0.0, reach, origin, direction),
    );
    while rayQueryProceed(&query) {}
    let hit = rayQueryGetCommittedIntersection(&query);
    if hit.kind == RAY_QUERY_INTERSECTION_NONE {
        return RayHit(-1.0, 0u);
    }
    return RayHit(hit.t, hit.instance_custom_data);
}

/// Which way what a ray hit faces there, against the ray: from two more
/// rays a hair beside it, the three points a small piece of its surface —
/// the hardware here does not hand back the triangle. Straight back along
/// the ray when the neighbours hit something else.
fn ray_facing(start: vec3<f32>, direction: vec3<f32>, first: RayHit, reach: f32, mask: u32) -> vec3<f32> {
    let hit = start + direction * first.t;
    let side = basis_of(direction);
    let apart = 0.0015 * first.t + 0.001;
    let a = ray_nearest(start + side.t * apart, direction, reach, mask);
    let b = ray_nearest(start + side.b * apart, direction, reach, mask);
    var n = -direction;
    if a.t > 0.0 && b.t > 0.0 && a.instance == first.instance && b.instance == first.instance {
        let pa = start + side.t * apart + direction * a.t;
        let pb = start + side.b * apart + direction * b.t;
        let c = cross(pa - hit, pb - hit);
        if dot(c, c) > 1e-16 {
            n = normalize(c);
            if dot(n, direction) > 0.0 {
                n = -n;
            }
        }
    }
    return n;
}

/// What is seen along a ray, lit: alpha 1 when it met something, 0 when
/// it went off into the sky. Glass is looked through.
fn ray_seen(start: vec3<f32>, direction: vec3<f32>) -> vec4<f32> {
    let reach = 300.0;
    let mask = RAY_THINGS | RAY_TERRAIN;
    let first = ray_nearest(start, direction, reach, mask);
    if first.t < 0.0 {
        return vec4<f32>(0.0);
    }
    let hit = start + direction * first.t;
    let n = ray_facing(start, direction, first, reach, mask);
    let m = ray_materials[first.instance];
    let albedo = m.color_metal.rgb;
    if m.color_metal.w < -0.5 {
        // A light itself: its colour and its glow.
        return vec4<f32>(albedo + m.emission_smooth.rgb, 1.0);
    }
    let metal = m.color_metal.w;
    let lifted = hit + n * 0.02;
    // The sky's hemisphere (or the irradiance volume's probes), the sun
    // with its shadow, the lamps with theirs.
    var light = ray_direct(hit, lifted, n);
    let volume = ddgi_irradiance(lifted, n, -direction);
    light += mix(mix(frame.ground_color.rgb, frame.sky_color.rgb, n.y * 0.5 + 0.5), volume.rgb, volume.a);
    var color = albedo * (1.0 - metal) * light + m.emission_smooth.rgb;
    // A metal seen this way: its own reflection, of the probes and sky.
    if metal > 0.0 {
        color += albedo * metal * probes_and_sky(hit, reflect(direction, n), 1.0 - m.emission_smooth.w);
    }
    return vec4<f32>(color, 1.0);
}

/// The sun and the lamps on a point a ray hit, each with its shadow ray
/// from `lifted`, just off it.
fn ray_direct(hit: vec3<f32>, lifted: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var light = vec3<f32>(0.0);
    let to_sun = -normalize(frame.sun_direction.xyz);
    let sun_facing = max(dot(n, to_sun), 0.0);
    if sun_facing > 0.0 {
        light += frame.sun_color.rgb * sun_facing * ray_visible(lifted, to_sun, 1.0e4);
    }
    let count = min(u32(frame.clusters.w), 64u);
    for (var i = 0u; i < count; i = i + 1u) {
        let lamp = lights[i];
        let to_lamp = lamp.position_range.xyz - hit;
        let distance_to = length(to_lamp);
        let reach_lamp = lamp_falloff(lamp, distance_to);
        if reach_lamp <= 0.0 {
            continue;
        }
        let toward = to_lamp / max(distance_to, 1e-4);
        let facing = max(dot(n, toward), 0.0);
        if facing <= 0.0 {
            continue;
        }
        let seen = ray_visible(lifted, toward, max(distance_to - 0.05, 0.0));
        light += lamp.color_shadow.rgb * facing * reach_lamp * seen;
    }
    return light;
}

/// The sky's light along a ray that met nothing, as the hemisphere
/// ambient has it: the scene's sky colour above (shaded by the physical
/// sky's picture, where there is one), the ground's below.
fn ddgi_sky(d: vec3<f32>) -> vec3<f32> {
    if d.y < 0.0 {
        return frame.ground_color.rgb;
    }
    var sky = frame.sky_color.rgb;
    if frame.air.x > 0.5 {
        sky = sky * physical_sky(d) / max(sky_toward(vec3<f32>(0.0, 1.0, 0.0)), vec3<f32>(1e-4));
    }
    return sky;
}

/// One of an irradiance volume's probes' rays this frame (ddgi.rs): what
/// it met, lit — by the sun, the lamps and the probes round it, so light
/// bounces on frame after frame — and how far; the distance negative for
/// the back of a face.
@compute @workgroup_size(64)
fn cs_ddgi_trace(@builtin(global_invocation_id) id: vec3<u32>) {
    let rays = u32(ddgi_step.rays.x);
    if id.x >= u32(ddgi_step.rays.w) * rays {
        return;
    }
    let probe = id.x / rays;
    let direction = ddgi_direction(id.x % rays);
    let origin = ddgi_position(probe);
    let reach = 300.0;
    let mask = RAY_THINGS | RAY_TERRAIN;
    var query: ray_query;
    rayQueryInitialize(&query, scene_rays, RayDesc(RAY_FLAG_NONE, mask, 0.0, reach, origin, direction));
    while rayQueryProceed(&query) {}
    let found = rayQueryGetCommittedIntersection(&query);
    if found.kind == RAY_QUERY_INTERSECTION_NONE {
        ddgi_rays[id.x] = vec4<f32>(ddgi_sky(direction), 1.0e4);
        return;
    }
    if !found.front_face {
        ddgi_rays[id.x] = vec4<f32>(0.0, 0.0, 0.0, -found.t);
        return;
    }
    let first = RayHit(found.t, found.instance_custom_data);
    let hit = origin + direction * found.t;
    let n = ray_facing(origin, direction, first, reach, mask);
    let m = ray_materials[first.instance];
    if m.color_metal.w < -0.5 {
        ddgi_rays[id.x] = vec4<f32>(m.color_metal.rgb + m.emission_smooth.rgb, found.t);
        return;
    }
    let lifted = hit + n * 0.02;
    var light = ray_direct(hit, lifted, n);
    let volume = ddgi_irradiance(lifted, n, -direction);
    light += mix(mix(frame.ground_color.rgb, frame.sky_color.rgb, n.y * 0.5 + 0.5), volume.rgb, volume.a);
    let color = m.color_metal.rgb * (1.0 - max(m.color_metal.w, 0.0)) * light + m.emission_smooth.rgb;
    ddgi_rays[id.x] = vec4<f32>(color, found.t);
}

/// What a mirror at `origin` sees along `direction`, lit: alpha 1 when the
/// ray met something, 0 when it went off into the sky (which the caller
/// has already).
fn ray_reflection(origin: vec3<f32>, direction: vec3<f32>) -> vec4<f32> {
    return ray_seen(origin + direction * 0.015, direction);
}

/// What is seen through glass at `position`, looking along `incoming`
/// into its face turned `normal` toward the eye: bent in, out through the
/// far side, and on — always alpha 1, the sky where nothing is met.
fn ray_refraction(position: vec3<f32>, incoming: vec3<f32>, normal: vec3<f32>, ior: f32) -> vec4<f32> {
    var d = refract(incoming, normal, 1.0 / ior);
    if dot(d, d) < 0.25 {
        d = reflect(incoming, normal);
    }
    var origin = position - normal * 0.01;
    // Its far side: the nearest glass on from inside it.
    let far = ray_nearest(origin, d, 50.0, RAY_GLASS);
    if far.t > 0.0 {
        let n = ray_facing(origin, d, far, 50.0, RAY_GLASS);
        let out = origin + d * far.t;
        var bent = refract(d, n, ior);
        if dot(bent, bent) < 0.25 {
            // Past the critical angle: turned back inside, and out the way
            // it came, near enough.
            bent = reflect(d, n);
        }
        origin = out + bent * 0.01;
        d = bent;
    }
    let seen = ray_seen(origin, d);
    if seen.a > 0.5 {
        return seen;
    }
    return vec4<f32>(probes_and_sky(origin, d, 0.0), 1.0);
}

// ReSTIR DI (restir.rs): each pixel's reservoir — one lamp, chosen in
// proportion to its unshadowed light — from candidates, last frame and
// neighbours, then one ray to it.

@group(3) @binding(10) var restir_depth: texture_depth_2d;
@group(3) @binding(11) var restir_normals: texture_2d<f32>;
@group(3) @binding(12) var<storage, read_write> restir_candidates: array<vec4<f32>>;
@group(3) @binding(13) var<storage, read> restir_before: array<vec4<f32>>;
@group(3) @binding(14) var<storage, read> restir_before_surface: array<vec4<f32>>;
@group(3) @binding(15) var<storage, read_write> restir_now: array<vec4<f32>>;
@group(3) @binding(16) var<storage, read_write> restir_surface: array<vec4<f32>>;

/// How much unshadowed light lamp `i` gives a surface at `p` turned `n`,
/// as one number: what the lamp is chosen by.
fn restir_target(i: u32, p: vec3<f32>, n: vec3<f32>) -> f32 {
    let light = lights[i];
    let to_light = light.position_range.xyz - p;
    let distance_to = length(to_light);
    let toward = to_light / max(distance_to, 1e-4);
    let reach = lamp_falloff(light, distance_to);
    let facing = max(dot(n, toward), 0.0);
    let along = dot(-toward, light.spot.xyz);
    let cone = spot_cone(light, along);
    let c = light.color_shadow.rgb;
    return (0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b) * facing * reach * cone;
}

/// A random number in 0..1 from a state, stepped (PCG).
fn restir_random(state: ptr<function, u32>) -> f32 {
    *state = *state * 747796405u + 2891336453u;
    var word = ((*state >> ((*state >> 28u) + 4u)) ^ *state) * 277803737u;
    word = (word >> 22u) ^ word;
    return f32(word) / 4294967296.0;
}

struct RestirSurface {
    position: vec3<f32>,
    normal: vec3<f32>,
    distance: f32,
};

fn restir_surface_at(pixel: vec2<u32>) -> RestirSurface {
    var out: RestirSurface;
    let d = textureLoad(restir_depth, pixel, 0);
    out.distance = -1.0;
    if d >= 1.0 {
        return out;
    }
    let size = vec2<f32>(frame.restir.yz);
    let uv = (vec2<f32>(pixel) + 0.5) / size;
    let seen = frame.inverse_view_projection * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    out.position = seen.xyz / seen.w;
    out.normal = normalize(textureLoad(restir_normals, pixel, 0).xyz);
    out.distance = distance(out.position, frame.camera_position.xyz);
    return out;
}

/// The same surface, near enough to lend its reservoir.
fn restir_alike(a: vec4<f32>, normal: vec3<f32>, distance_to: f32) -> bool {
    return a.w > 0.0 && abs(a.w - distance_to) < 0.1 * distance_to && dot(a.xyz, normal) > 0.9;
}

@compute @workgroup_size(8, 8)
fn cs_restir_initial(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<u32>(frame.restir.yz);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let at = id.y * size.x + id.x;
    let surface = restir_surface_at(id.xy);
    if surface.distance < 0.0 {
        restir_candidates[at] = vec4<f32>(0.0);
        restir_surface[at] = vec4<f32>(0.0, 0.0, 0.0, -1.0);
        return;
    }
    restir_surface[at] = vec4<f32>(surface.normal, surface.distance);
    var state = at * 9781u + u32(frame.restir.w) * 6271u + 1u;
    let cell = light_cells[light_cell(vec2<f32>(id.xy) + 0.5, surface.position)];
    let count = cell.y;
    var chosen = 0u;
    var sum = 0.0;
    var m = 0.0;
    if count > 0u {
        let candidates = min(count, 8u);
        for (var k = 0u; k < candidates; k = k + 1u) {
            let i = light_indices[cell.x + min(u32(restir_random(&state) * f32(count)), count - 1u)];
            let w = restir_target(i, surface.position, surface.normal) * f32(count);
            sum += w;
            if restir_random(&state) * sum < w {
                chosen = i;
            }
        }
        m = f32(candidates);
    }
    // Where it was last frame, if it was this surface: its reservoir, at
    // most twenty times this frame's.
    let before = frame.previous_view_projection * vec4<f32>(surface.position, 1.0);
    if before.w > 1e-4 && frame.restir.w > 0.5 {
        let ndc = before.xy / before.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if all(uv >= vec2<f32>(0.0)) && all(uv < vec2<f32>(1.0)) {
            let pixel = vec2<u32>(uv * vec2<f32>(size));
            let there = pixel.y * size.x + pixel.x;
            if restir_alike(restir_before_surface[there], surface.normal, surface.distance) {
                let old = restir_before[there];
                let light = bitcast<u32>(old.x);
                let kept = min(old.z, 20.0 * max(m, 1.0));
                let w = restir_target(light, surface.position, surface.normal) * old.w * kept;
                sum += w;
                if restir_random(&state) * sum < w {
                    chosen = light;
                }
                m += kept;
            }
        }
    }
    let target_now = restir_target(chosen, surface.position, surface.normal);
    let weight = select(0.0, sum / (m * target_now), target_now > 0.0 && m > 0.0);
    restir_candidates[at] = vec4<f32>(bitcast<f32>(chosen), sum, m, weight);
}

@compute @workgroup_size(8, 8)
fn cs_restir_spatial(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<u32>(frame.restir.yz);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let at = id.y * size.x + id.x;
    let here = restir_surface[at];
    if here.w < 0.0 {
        restir_now[at] = vec4<f32>(0.0);
        return;
    }
    let surface = restir_surface_at(id.xy);
    var state = at * 7919u + u32(frame.restir.w) * 104729u + 7u;
    let own = restir_candidates[at];
    var chosen = bitcast<u32>(own.x);
    var sum = restir_target(chosen, surface.position, surface.normal) * own.w * own.z;
    var m = own.z;
    for (var k = 0u; k < 3u; k = k + 1u) {
        let a = restir_random(&state) * 6.2831853;
        let r = sqrt(restir_random(&state)) * 16.0;
        let offset = vec2<i32>(i32(cos(a) * r), i32(sin(a) * r));
        let pixel = vec2<i32>(id.xy) + offset;
        if any(pixel < vec2<i32>(0)) || any(pixel >= vec2<i32>(size)) {
            continue;
        }
        let there = u32(pixel.y) * size.x + u32(pixel.x);
        if !restir_alike(restir_surface[there], surface.normal, surface.distance) {
            continue;
        }
        let other = restir_candidates[there];
        let light = bitcast<u32>(other.x);
        let w = restir_target(light, surface.position, surface.normal) * other.w * other.z;
        sum += w;
        if restir_random(&state) * sum < w {
            chosen = light;
        }
        m += other.z;
    }
    let target_now = restir_target(chosen, surface.position, surface.normal);
    var weight = select(0.0, sum / (m * target_now), target_now > 0.0 && m > 0.0);
    // The one ray: to the lamp chosen, at a point of its ball.
    if weight > 0.0 {
        let light = lights[chosen];
        var aim = light.position_range.xyz;
        let size_of = frame.ray_params.w;
        if size_of > 0.0 {
            let side = basis_of(normalize(aim - surface.position));
            let a = restir_random(&state) * 6.2831853;
            aim = aim + (side.t * cos(a) + side.b * sin(a)) * sqrt(restir_random(&state)) * size_of;
        }
        let start = surface.position + surface.normal * 0.02;
        let to_aim = aim - start;
        let aim_distance = length(to_aim);
        weight *= ray_visible(start, to_aim / max(aim_distance, 1e-4), max(aim_distance - size_of - 0.05, 0.0));
    }
    restir_now[at] = vec4<f32>(bitcast<f32>(chosen), sum, min(m, 160.0), weight);
}
