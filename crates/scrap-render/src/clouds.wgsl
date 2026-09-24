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
    // the dust wall: density, its front's distance upwind now, its height,
    // 1 when there is a prepass depth to stop at
    dust: vec4<f32>,
    // the camera view's third row: how deep a point is
    view_depth: vec4<f32>,
    // near, far; which way the wind blows, level
    depth_range: vec4<f32>,
    // the dust volume round the camera: its corner's x and z, a cell's
    // width and its height, in metres
    dust_box: vec4<f32>,
    // how many dust devils and crest plumes
    local: vec4<f32>,
    // each devil: foot and radius; height, strength, spin
    devils: array<vec4<f32>, 12>,
    // each plume: middle on the crest and half its length; the crest's
    // way and how much is blowing
    plumes: array<vec4<f32>, 128>,
};

// The dust wall. Its billows are a tileable Worley noise made once
// (`cs_dust_noise`, 128³, four cells a tile) and read wherever the march
// goes, at full detail; its light — how much sun comes through, how open
// to the sky — is worked out each frame into a coarse volume round the
// camera (`cs_dust_light`), which is all light needs.
const DUST_CELLS = vec3<u32>(128u, 32u, 128u);
const NOISE_SIZE = 128u;
/// Worley cells along each tile of the noise.
const NOISE_CELLS = 4.0;
@group(1) @binding(0) var dust_noise_out: texture_storage_3d<rgba8unorm, write>;
@group(1) @binding(1) var dust_noise: texture_3d<f32>;
@group(1) @binding(2) var dust_lit_out: texture_storage_3d<rgba16float, write>;
@group(1) @binding(3) var dust_lit: texture_3d<f32>;
@group(1) @binding(4) var dust_sampler: sampler;
@group(1) @binding(5) var noise_sampler: sampler;

fn dust_cell_centre(id: vec3<u32>) -> vec3<f32> {
    let b = cloud.dust_box;
    return vec3<f32>(b.x + (f32(id.x) + 0.5) * b.z, (f32(id.y) + 0.5) * b.w, b.y + (f32(id.z) + 0.5) * b.z);
}

/// Where a point falls in the volume, 0 to 1 each way.
fn dust_uvw(p: vec3<f32>) -> vec3<f32> {
    let b = cloud.dust_box;
    return vec3<f32>(
        (p.x - b.x) / (b.z * f32(DUST_CELLS.x)),
        p.y / (b.w * f32(DUST_CELLS.y)),
        (p.z - b.y) / (b.z * f32(DUST_CELLS.z)),
    );
}

fn inside01(u: vec3<f32>) -> bool {
    return all(u >= vec3<f32>(0.0)) && all(u <= vec3<f32>(1.0));
}

/// Worley at `q` (in its cells), wrapping every `period` cells: one minus
/// the distance to the nearest of points scattered one to a cell.
fn tiled_puffs(q: vec3<f32>, period: f32) -> f32 {
    let cell = floor(q);
    var nearest = 1.0e3;
    for (var z = -1; z <= 1; z = z + 1) {
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let c = cell + vec3<f32>(f32(x), f32(y), f32(z));
                let w = c - floor(c / period) * period;
                let point = c + vec3<f32>(hash3(w), hash3(w + 17.3), hash3(w + 41.9));
                nearest = min(nearest, length(q - point));
            }
        }
    }
    return clamp(1.0 - nearest, 0.0, 1.0);
}

// The noise, once: Worley at four cells a tile in r, eight in g, sixteen
// in b — the wall's billows, and the fine curls that eat its edges.
@compute @workgroup_size(4, 4, 4)
fn cs_dust_noise(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id >= vec3<u32>(NOISE_SIZE)) {
        return;
    }
    let u = (vec3<f32>(id) + 0.5) / f32(NOISE_SIZE);
    textureStore(dust_noise_out, id, vec4<f32>(
        tiled_puffs(u * NOISE_CELLS, NOISE_CELLS),
        tiled_puffs(u * NOISE_CELLS * 2.0, NOISE_CELLS * 2.0),
        tiled_puffs(u * NOISE_CELLS * 4.0, NOISE_CELLS * 4.0),
        1.0,
    ));
}

/// The billows of a size: `q` in the billows' own units, as `puffs` took.
fn billows(q: vec3<f32>) -> f32 {
    return textureSampleLevel(dust_noise, noise_sampler, q / NOISE_CELLS, 0.0).r;
}

/// The fine curls, `q` in metres over their size.
fn curls(q: vec3<f32>) -> vec2<f32> {
    return textureSampleLevel(dust_noise, noise_sampler, q / NOISE_CELLS, 0.0).gb;
}

// The light in each cell: how much sun comes through the dust towards it,
// how much light turned many times reaches it, and how open to the sky it
// is — a billow standing out, or a fold between two, or under a tier.
@compute @workgroup_size(8, 8, 4)
fn cs_dust_light(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id >= DUST_CELLS) {
        return;
    }
    let p = dust_cell_centre(id);
    let density = dust_density(p, false);
    let to_sun = cloud.to_sun.xyz;
    var above = 0.0;
    var previous = 0.0;
    for (var j = 1; j <= 6; j = j + 1) {
        let s = f32(j) * f32(j) * 12.0;
        above += dust_density(p + to_sun * s, false) * (s - previous);
        previous = s;
    }
    let step = cloud.dust_box.z * 0.8;
    let over = dust_density(p + vec3<f32>(0.0, 35.0, 0.0), false);
    let round = (dust_density(p + vec3<f32>(step, 0.0, 0.0), false) + dust_density(p - vec3<f32>(step, 0.0, 0.0), false)
        + dust_density(p + vec3<f32>(0.0, 0.0, step), false) + dust_density(p - vec3<f32>(0.0, 0.0, step), false)) * 0.25;
    let open = exp(-(over + round) * 1.2);
    textureStore(dust_lit_out, id, vec4<f32>(density, exp(-above * 0.05), open, exp(-above * 0.006)));
}


@group(0) @binding(0) var<uniform> cloud: Cloud;
@group(0) @binding(1) var clouds_out: texture_storage_2d<rgba16float, write>;
// The solid scene's depth: rays stop at what stands in front.
@group(0) @binding(2) var scene_depth: texture_depth_2d;

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

/// Which way the wind blows, level.
fn wind_way() -> vec2<f32> {
    let w = cloud.depth_range.zw;
    return select(vec2<f32>(1.0, 0.0), normalize(w), length(w) > 1e-4);
}

/// Rounded masses: 1 at the heart of a ball, falling to 0 at its rim —
/// one minus the distance to the nearest of points scattered one to a
/// cell (Worley). What makes a cloud's cauliflower, where plain noise
/// makes mush.
fn puffs(p: vec3<f32>) -> f32 {
    let cell = floor(p);
    var nearest = 1.0e3;
    for (var z = -1; z <= 1; z = z + 1) {
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let c = cell + vec3<f32>(f32(x), f32(y), f32(z));
                let point = c + vec3<f32>(hash3(c), hash3(c + 17.3), hash3(c + 41.9));
                nearest = min(nearest, length(p - point));
            }
        }
    }
    return clamp(1.0 - nearest, 0.0, 1.0);
}

/// How much dust the wall holds at a point: a dense mass behind a front
/// upwind, its face and top heaped with rounded billows of three sizes, its
/// top rising in tiers the deeper into it — low in front, higher behind,
/// highest at the back — and hard-edged, as a haboob is: a solid thing,
/// not a haze. `detail` adds the smallest billows.
fn dust_density(p: vec3<f32>, detail: bool) -> f32 {
    return dust_field(p, detail).x;
}

/// The wall at a point: its density, and how far inside it the point is
/// (metres; below 0 outside) — how far a ray may step and not miss it.
fn dust_field(p: vec3<f32>, detail: bool) -> vec2<f32> {
    let w = wind_way();
    let along = dot(p.xz, w);
    let across = dot(p.xz, vec2<f32>(-w.y, w.x));
    let t = cloud.eye.w;
    let height = cloud.dust.z;
    if p.y > height * 1.35 || p.y < -5.0 {
        return vec2<f32>(0.0, -max(p.y - height * 1.35, 0.0) - 1.0);
    }
    let behind = -cloud.dust.y - along;
    // Ahead of the front: the dust it throws before it, low on the ground
    // and thickening toward it — the air browning before the wall comes.
    let ahead = max(-behind, 0.0);
    // Heaped as the wall is, not a flat layer: its depth goes up and down
    // with the billows over it.
    // (Only low, and not far before it: above and beyond, none to speak of.)
    var haze = 0.0;
    if p.y < 200.0 && ahead < 900.0 {
        let heap = billows(vec3<f32>(across, 0.0, along - t * 4.0) / 90.0 + 5.3);
        haze = 0.012 * exp(-ahead / 160.0) * exp(-max(p.y, 0.0) / (10.0 + 40.0 * heap));
    }
    if behind < -260.0 {
        return vec2<f32>(haze, min(-(ahead - 260.0), -1.0));
    }
    // The billows roll: carried a little faster than the front, and up.
    let rolling = vec3<f32>(across, p.y - t * 1.5, along - t * 3.0);
    var shape = billows(rolling / 260.0) * 0.55 + billows(rolling / 110.0 + 3.1) * 0.3;
    if detail {
        shape += billows(rolling / 45.0 + 7.7) * 0.15;
    } else {
        shape += 0.07;
    }
    // Its face: where the billows push out past the front's line.
    let face = behind + (shape - 0.5) * 360.0;
    // Its top, in tiers set back one behind another.
    let tiers = 0.42
        + 0.28 * smoothstep(90.0, 150.0, behind + (shape - 0.5) * 120.0)
        + 0.30 * smoothstep(330.0, 420.0, behind + (shape - 0.5) * 160.0);
    let top = height * tiers + (shape - 0.5) * height * 0.55;
    var inside = min(face, top - p.y);
    // The fine curls eat into its surface: billows break into billows, and
    // no round ball keeps its outline.
    if detail && inside > -40.0 && inside < 60.0 {
        let c = curls(rolling / 38.0 + vec3<f32>(0.0, -t * 0.02, 0.0));
        inside -= (1.0 - c.x) * 26.0 + (1.0 - c.y) * 9.0;
    }
    // Hard-edged, dense all through: its foot the densest.
    let foot = 1.0 + 0.8 * exp(-max(p.y, 0.0) / (height * 0.2));
    var body = clamp(inside / 8.0, 0.0, 1.0) * foot;
    // Inside, it is not even: billows within billows, gaps between them —
    // what the eye goes through from within, or close. As dense on the
    // whole as elsewhere.
    // Only near: from far the parcels are too small to see, and an even
    // body goes opaque in fewer steps.
    if detail && inside > 0.0 && distance(p, cloud.eye.xyz) < 250.0 {
        let within = billows(rolling / 32.0 + vec3<f32>(1.7, t * 0.15, 4.2));
        body *= 0.25 + 1.5 * within * within;
    }
    return vec2<f32>(max(body, haze) * cloud.dust.x, inside);
}

fn henyey(cos: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (12.566371 * pow(max(1.0 + g2 - 2.0 * g * cos, 1e-4), 1.5));
}

// Dust near the ground, marched where the ray crosses it: devils and sand
// off dune crests. Light and what is let through, in front of the rest.
struct Local {
    light: vec3<f32>,
    through: f32,
};

fn downwind() -> vec3<f32> {
    let w = wind_way();
    return vec3<f32>(w.x, 0.0, w.y);
}

// A devil's sand at `p`: a turning funnel flaring as it rises, leaning
// downwind, streaked by its own turning; a skirt of dust round its foot.
fn devil_density(i: u32, p: vec3<f32>) -> f32 {
    let foot = cloud.devils[2u * i];
    let more = cloud.devils[2u * i + 1u];
    let height = more.x;
    let up = p.y - foot.y;
    if up < -0.5 || up > height {
        return 0.0;
    }
    let time = cloud.eye.w;
    let rise = clamp(up / height, 0.0, 1.0);
    let centre = foot.xyz + downwind() * up * 0.22;
    let off = p.xz - centre.xz;
    let r = length(off);
    // Narrow at the ground, opening like a trumpet higher up.
    let flare = foot.w * (0.3 + 1.6 * pow(rise, 1.3));
    let turn = atan2(off.y, off.x) * more.z + time * 3.2 - up * 0.18;
    // Strands wound round it, climbing as it turns.
    let strands = noise3(vec3<f32>(cos(turn) * 3.5, sin(turn) * 3.5, up * 0.22 - time * 2.4));
    let grain = noise3(vec3<f32>(cos(turn) * 9.0, sin(turn) * 9.0, up * 0.9 - time * 4.0));
    let wall = exp(-pow((r - flare * 0.7) / (flare * 0.35 + 0.3), 2.0));
    let body = smoothstep(0.0, 1.0, up) * (1.0 - smoothstep(0.35, 1.0, rise));
    var d = wall * body * max(strands * 2.2 - 0.6 + grain * 0.5, 0.0);
    // The skirt: dust thrown out round its foot, low and ragged.
    let skirt = exp(-max(up, 0.0) / 1.0) * exp(-pow(r / (foot.w * 2.2), 2.0));
    d += 0.9 * skirt * max(strands + grain - 0.5, 0.0);
    return more.y * d;
}

// Sand off a crest at `p`: a sheet streaming downwind, lifting a little
// and thinning as it goes, in streaks.
fn plume_density(i: u32, p: vec3<f32>) -> f32 {
    let at = cloud.plumes[2u * i];
    let way = cloud.plumes[2u * i + 1u];
    let rel = p - at.xyz;
    let s = dot(rel, way.xyz);
    let dd = dot(rel, downwind());
    let centre = 0.15 + 0.32 * dd;
    let thick = 0.25 + 0.22 * max(dd, 0.0);
    let sheet = exp(-pow((rel.y - centre) / thick, 2.0));
    let edge = 1.0 - smoothstep(at.w * 0.5, at.w * 1.3, abs(s));
    let fade = exp(-max(dd, 0.0) / 4.0) * smoothstep(-0.5, 0.2, dd);
    // In the world's own coordinates, so the streaks of one stretch of
    // crest run on into the next.
    let wd = downwind();
    let streak = noise3(vec3<f32>(dot(p.xz, vec2<f32>(-wd.z, wd.x)) * 0.9, p.y * 2.5, dot(p.xz, wd.xz) * 0.7 - cloud.eye.w * 5.0));
    return way.w * 1.1 * sheet * fade * edge * max(streak * 1.8 - 0.5, 0.0);
}

// Where a ray crosses a box: `centre`, its three half-axes as vectors.
fn box_span(eye: vec3<f32>, d: vec3<f32>, centre: vec3<f32>, ax: vec3<f32>, ay: vec3<f32>, az: vec3<f32>) -> vec2<f32> {
    var t0 = -1.0e9;
    var t1 = 1.0e9;
    let rel = eye - centre;
    for (var k = 0; k < 3; k = k + 1) {
        var axis = ax;
        if k == 1 {
            axis = ay;
        }
        if k == 2 {
            axis = az;
        }
        let len2 = dot(axis, axis);
        let o = dot(rel, axis) / len2;
        let v = dot(d, axis) / len2;
        if abs(v) < 1e-6 {
            if abs(o) > 1.0 {
                return vec2<f32>(1.0, 0.0);
            }
            continue;
        }
        let a = (-1.0 - o) / v;
        let b = (1.0 - o) / v;
        t0 = max(t0, min(a, b));
        t1 = min(t1, max(a, b));
    }
    return vec2<f32>(t0, t1);
}

fn local_dust(eye: vec3<f32>, d: vec3<f32>, reach: f32, jitter: f32) -> Local {
    var out: Local;
    out.light = vec3<f32>(0.0);
    out.through = 1.0;
    let sand = vec3<f32>(0.9, 0.66, 0.44);
    // Lit by the sun as a thin dust is — brighter looking toward it — and
    // by the sky and the sand round it.
    let toward = max(dot(d, cloud.to_sun.xyz), 0.0);
    let lit = (cloud.sun.rgb * cloud.to_sun.w * (0.75 + 1.2 * pow(toward, 4.0)) + cloud.ambient.rgb * 0.8) * sand;
    let up = vec3<f32>(0.0, 1.0, 0.0);
    for (var i = 0u; i < u32(cloud.local.x); i = i + 1u) {
        let foot = cloud.devils[2u * i];
        let more = cloud.devils[2u * i + 1u];
        let wide = foot.w * 2.6 + 3.0 + more.x * 0.25;
        let centre = foot.xyz + vec3<f32>(0.0, more.x * 0.5, 0.0) + downwind() * more.x * 0.11;
        let span = box_span(eye, d, centre, vec3<f32>(wide, 0.0, 0.0), up * more.x * 0.52, vec3<f32>(0.0, 0.0, wide));
        let t0 = max(span.x, 0.0);
        let t1 = min(span.y, reach);
        if t1 <= t0 {
            continue;
        }
        let steps = 24;
        let step = (t1 - t0) / f32(steps);
        for (var k = 0; k < steps; k = k + 1) {
            let p = eye + d * (t0 + step * (f32(k) + jitter));
            let density = devil_density(i, p);
            if density <= 0.002 {
                continue;
            }
            let passed = exp(-density * 0.6 * step);
            out.light += out.through * lit * (1.0 - passed);
            out.through *= passed;
        }
    }
    for (var i = 0u; i < u32(cloud.local.y); i = i + 1u) {
        let at = cloud.plumes[2u * i];
        let way = cloud.plumes[2u * i + 1u];
        let w = downwind();
        // The box the sheet can reach: along the crest, up to where it
        // has risen and spread, downwind to where it has thinned away.
        let centre = at.xyz + w * 5.75 + up * 3.0;
        // Square axes (the slab test wants them so): along the crest, and
        // across it toward downwind, wide enough for a wind at a slant.
        let crest = normalize(way.xyz - up * dot(way.xyz, up));
        var across = normalize(cross(up, crest));
        if dot(across, w) < 0.0 {
            across = -across;
        }
        let slant = abs(dot(w, crest));
        let span = box_span(
            eye,
            d,
            centre,
            crest * (at.w * 1.3 + 6.25 * slant),
            up * 4.5,
            across * 6.25,
        );
        let t0 = max(span.x, 0.0);
        let t1 = min(span.y, reach);
        if t1 <= t0 {
            continue;
        }
        let steps = 12;
        let step = (t1 - t0) / f32(steps);
        for (var k = 0; k < steps; k = k + 1) {
            let p = eye + d * (t0 + step * (f32(k) + jitter));
            let density = plume_density(i, p);
            if density <= 0.002 {
                continue;
            }
            let passed = exp(-density * 1.2 * step);
            out.light += out.through * lit * (1.0 - passed);
            out.through *= passed;
        }
    }
    return out;
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

    // How far the ray may go: to what the scene has there, if anything.
    var reach = 1.0e9;
    if cloud.dust.w > 0.5 {
        let full = vec2<i32>(textureDimensions(scene_depth));
        let pixel = clamp(vec2<i32>(uv * vec2<f32>(full)), vec2<i32>(0), full - vec2<i32>(1));
        let z = textureLoad(scene_depth, pixel, 0);
        if z < 1.0 {
            let near = cloud.depth_range.x;
            let far = cloud.depth_range.y;
            let view = near * far / (far - z * (far - near));
            let per_metre = max(-dot(cloud.view_depth.xyz, d), 1e-4);
            reach = view / per_metre;
        }
    }

    // Devils and crest plumes: nearest of all, in front of the rest.
    var near = Local(vec3<f32>(0.0), 1.0);
    if cloud.local.x + cloud.local.y > 0.0 {
        let jit = 0.3 + 0.4 * fract(sin(dot(vec2<f32>(id.xy), vec2<f32>(39.346, 11.135))) * 43758.547);
        near = local_dust(eye, d, reach, jit);
    }

    // The dust wall first: it is on the ground, nearer than any cloud.
    var dust_light = vec3<f32>(0.0);
    var dust_through = 1.0;
    if cloud.dust.x > 0.0 {
        let w = wind_way();
        let along0 = dot(eye.xz, w);
        let along_d = dot(d.xz, w);
        // Its bulges and billows reach up to 240 m either side of it, and
        // the dust it throws before it a few hundred more.
        let front = -cloud.dust.y + 900.0;
        var t0 = 0.0;
        var t1 = -1.0;
        // Into the mass no further than it takes to go opaque, twice over.
        if along0 < front {
            t1 = 2200.0;
        } else if along_d < -1e-4 {
            t0 = (along0 - front) / -along_d;
            t1 = t0 + 2200.0;
        }
        if d.y > 1e-4 {
            t1 = min(t1, (cloud.dust.z * 1.5 - eye.y) / d.y);
        }
        t1 = min(t1, reach);
        if t1 > t0 {
            // Steps sized by how far the wall is: long through clear air,
            // short in and near it, where its edges are.
            // Noise in where the steps fall hides their banding: little
            // when nothing averages it over frames, all of a step when TAA
            // does (it turns each frame, `size.z`), and the steps longer.
            let turning = cloud.size.z > 0.0;
            let fine = select(5.0, 8.0, turning);
            let still = fract(sin(dot(vec2<f32>(id.xy), vec2<f32>(12.9898, 78.233))) * 43758.547);
            let jitter = select(0.3 + 0.4 * still, fract(still + cloud.size.z), turning);
            // What dust gives back of the light, by colour: the sun's once,
            // and the light turned many times — each turn redder and less.
            let sand = vec3<f32>(0.92, 0.62, 0.36);
            let deep = vec3<f32>(0.62, 0.36, 0.18);
            var t = t0 + fine * jitter;
            for (var i = 0; i < 200; i = i + 1) {
                if t >= t1 {
                    break;
                }
                let p = eye + d * t;
                let field = dust_field(p, true);
                let density = field.x;
                // Outside it, as far as it is and never less than a few
                // fine steps; inside, fine.
                var step = fine;
                if field.y < 0.0 {
                    step = clamp(-field.y * 0.8, fine * 1.6, 80.0);
                }
                t += step;
                if density <= 0.002 {
                    continue;
                }
                let u = dust_uvw(p);
                // Past the light volume (a few km off): in the wall, as dim as
                // its depths; before it, in the sun.
                let deep_in = smoothstep(-40.0, 120.0, field.y);
                var cell = vec4<f32>(density, 1.0 - 0.95 * deep_in, 1.0 - 0.6 * deep_in, 1.0 - 0.8 * deep_in);
                if inside01(u) {
                    cell = textureSampleLevel(dust_lit, dust_sampler, u, 0.0);
                }
                let low = clamp(p.y / cloud.dust.z, 0.0, 1.0);
                let open = cell.b;
                // Deep inside, light scattered through the dust from all
                // round: never black, and the same glow the storm has when
                // it closes over the camera.
                let many = (cell.a * 0.45 + 0.1) * (0.6 + 0.5 * open) * (0.55 + 0.45 * low);
                // A thick parcel shades itself, a thin one lets the light
                // through: the dust's swirls, from within as from without.
                let self_shade = mix(1.3, 0.55, clamp(density / 1.8, 0.0, 1.0));
                let lit = (cloud.sun.rgb * cloud.to_sun.w * (cell.g * 1.1 * sand + many * deep)
                    + cloud.ambient.rgb * (0.3 + 0.6 * low) * (0.5 + 0.8 * open) * deep) * self_shade;
                let extinction = density * 0.07;
                let passed = exp(-extinction * step);
                dust_light += dust_through * lit * (1.0 - passed);
                dust_through *= passed;
                if dust_through < 0.01 {
                    break;
                }
            }
        }
    }
    if dust_through < 0.01 || reach < 1.0e8 {
        // Nothing beyond: the wall hides it, or the scene does.
        textureStore(clouds_out, id.xy, vec4<f32>(near.light + near.through * dust_light, near.through * dust_through));
        return;
    }

    // Where the ray is inside the slab.
    let base = cloud.shape.y;
    let top = base + cloud.shape.z;
    var result = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if cloud.shape.x <= 0.0 || abs(d.y) < 1e-4 {
        textureStore(clouds_out, id.xy, vec4<f32>(near.light + near.through * dust_light, near.through * dust_through));
        return;
    }
    let t_base = (base - eye.y) / d.y;
    let t_top = (top - eye.y) / d.y;
    let t0 = max(min(t_base, t_top), 0.0);
    let t1 = min(max(t_base, t_top), cloud.drift.w);
    if t1 <= t0 {
        textureStore(clouds_out, id.xy, vec4<f32>(near.light + near.through * dust_light, near.through * dust_through));
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
    // The dust in front of the clouds.
    let behind = vec4<f32>(dust_light + dust_through * result.rgb, dust_through * result.a);
    textureStore(clouds_out, id.xy, vec4<f32>(near.light + near.through * behind.rgb, near.through * behind.a));
}
