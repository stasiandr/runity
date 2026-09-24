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
    // The sky's hemisphere, the sun with its shadow, the lamps with theirs.
    var light = mix(frame.ground_color.rgb, frame.sky_color.rgb, n.y * 0.5 + 0.5);
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
        let reach_lamp = clamp(1.0 - distance_to / lamp.position_range.w, 0.0, 1.0);
        if reach_lamp <= 0.0 {
            continue;
        }
        let toward = to_lamp / max(distance_to, 1e-4);
        let facing = max(dot(n, toward), 0.0);
        if facing <= 0.0 {
            continue;
        }
        let seen = ray_visible(lifted, toward, max(distance_to - 0.05, 0.0));
        light += lamp.color_shadow.rgb * facing * reach_lamp * reach_lamp * seen;
    }
    var color = albedo * (1.0 - metal) * light + m.emission_smooth.rgb;
    // A metal seen this way: its own reflection, of the probes and sky.
    if metal > 0.0 {
        color += albedo * metal * probes_and_sky(hit, reflect(direction, n), 1.0 - m.emission_smooth.w);
    }
    return vec4<f32>(color, 1.0);
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
