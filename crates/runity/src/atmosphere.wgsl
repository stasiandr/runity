// The physical sky's compute passes (atmosphere.rs): a picture of the whole
// sky from where the camera stands, and a grid over the view of what the
// air between the eye and each cell adds and lets through. The same model
// as the CPU's, in metres over a round planet.

struct Air {
    inverse_view_projection: mat4x4<f32>,
    // towards the sun; w the camera's altitude
    to_sun: vec4<f32>,
    // the camera's position; w the aerial grid's far end in metres
    eye: vec4<f32>,
    // rayleigh, mie, ozone amounts; haze anisotropy
    amounts: vec4<f32>,
    // brightness, the sun's intensity, the aerial scale, ground albedo
    scale: vec4<f32>,
};

@group(0) @binding(0) var<uniform> air: Air;
@group(0) @binding(1) var sky_view_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var aerial_out: texture_storage_3d<rgba16float, write>;

const PLANET: f32 = 6360000.0;
const TOP: f32 = 6460000.0;
const RAYLEIGH: vec3<f32> = vec3<f32>(5.802e-6, 13.558e-6, 33.1e-6);
const MIE_SCATTER: f32 = 3.996e-6;
const MIE_ABSORB: f32 = 4.4e-6;
const OZONE: vec3<f32> = vec3<f32>(0.65e-6, 1.881e-6, 0.085e-6);
const PI: f32 = 3.14159265;
const SKY_VIEW: vec2<u32> = vec2<u32>(192u, 108u);
const AERIAL: u32 = 32u;

struct Medium {
    rayleigh: vec3<f32>,
    mie: f32,
    extinction: vec3<f32>,
};

fn medium(height: f32) -> Medium {
    let h = max(height, 0.0);
    let r = exp(-h / 8000.0) * air.amounts.x;
    let m = exp(-h / 1200.0) * air.amounts.y;
    let o = max(1.0 - abs(h - 25000.0) / 15000.0, 0.0) * air.amounts.z;
    return Medium(RAYLEIGH * r, MIE_SCATTER * m, RAYLEIGH * r + vec3<f32>((MIE_SCATTER + MIE_ABSORB) * m) + OZONE * o);
}

/// Along `d` from `o`, where the top of the air is (negative: never).
fn to_top(o: vec3<f32>, d: vec3<f32>) -> f32 {
    let b = dot(o, d);
    let c = dot(o, o) - TOP * TOP;
    let disc = b * b - c;
    if disc < 0.0 {
        return -1.0;
    }
    return -b + sqrt(disc);
}

/// Along `d` from `o`, where the ground is (negative: never).
fn to_ground(o: vec3<f32>, d: vec3<f32>) -> f32 {
    let b = dot(o, d);
    let c = dot(o, o) - PLANET * PLANET;
    let disc = b * b - c;
    if disc < 0.0 {
        return -1.0;
    }
    return -b - sqrt(disc);
}

/// How much sunlight reaches a point, from the top of the air.
fn sun_through(p: vec3<f32>) -> vec3<f32> {
    let d = air.to_sun.xyz;
    if to_ground(p, d) > 0.0 {
        return vec3<f32>(0.0);
    }
    let span = to_top(p, d);
    let steps = 12;
    let step = span / f32(steps);
    var depth = vec3<f32>(0.0);
    for (var i = 0; i < steps; i = i + 1) {
        let q = p + d * (step * (f32(i) + 0.5));
        depth += medium(length(q) - PLANET).extinction * step;
    }
    return exp(-depth);
}

fn rayleigh_phase(cos: f32) -> f32 {
    return 3.0 / (16.0 * PI) * (1.0 + cos * cos);
}

fn mie_phase(cos: f32, g: f32) -> f32 {
    let g2 = g * g;
    return 3.0 / (8.0 * PI) * ((1.0 - g2) * (1.0 + cos * cos))
        / ((2.0 + g2) * pow(max(1.0 + g2 - 2.0 * g * cos, 1e-4), 1.5));
}

struct March {
    light: vec3<f32>,
    through: vec3<f32>,
};

/// March from `o` along `d` for `span` metres: the light added on the
/// way and how much gets through.
fn march(o: vec3<f32>, d: vec3<f32>, span: f32, steps: i32) -> March {
    let cos = dot(d, air.to_sun.xyz);
    let pr = rayleigh_phase(cos);
    let pm = mie_phase(cos, air.amounts.w);
    let step = span / f32(steps);
    var through = vec3<f32>(1.0);
    var light = vec3<f32>(0.0);
    for (var i = 0; i < steps; i = i + 1) {
        let p = o + d * (step * (f32(i) + 0.5));
        let m = medium(length(p) - PLANET);
        let sun = sun_through(p);
        let scattering = m.rayleigh * pr + vec3<f32>(m.mie * pm);
        // Light scattered more than once, spread evenly.
        let many = (m.rayleigh + vec3<f32>(m.mie)) * 0.25 / (4.0 * PI);
        let added = (scattering * sun + many * length(sun) * 0.577) * air.scale.x;
        let segment = exp(-m.extinction * step);
        light += through * added * (vec3<f32>(1.0) - segment) / max(m.extinction, vec3<f32>(1e-12));
        through *= segment;
    }
    return March(light, through);
}

/// The sky table's direction for a texel: azimuth across, and up the
/// latitude squeezed towards the horizon, where the sky changes fastest.
fn sky_view_direction(uv: vec2<f32>) -> vec3<f32> {
    let azimuth = (uv.x - 0.5) * 2.0 * PI;
    let t = uv.y * 2.0 - 1.0;
    let latitude = sign(t) * t * t * (PI * 0.5);
    return vec3<f32>(cos(latitude) * cos(azimuth), sin(latitude), cos(latitude) * sin(azimuth));
}

@compute @workgroup_size(8, 8, 1)
fn cs_sky_view(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= SKY_VIEW.x || id.y >= SKY_VIEW.y {
        return;
    }
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(SKY_VIEW);
    let d = sky_view_direction(uv);
    let o = vec3<f32>(0.0, PLANET + max(air.to_sun.w, 1.0), 0.0);
    var span = to_top(o, d);
    let ground = to_ground(o, d);
    if ground > 0.0 {
        span = min(span, ground);
    }
    let result = march(o, d, span, 24);
    var light = result.light;
    // Below the horizon, the ground: lit by the sun and the sky, dimmed by
    // the air in front of it.
    if ground > 0.0 {
        let floor = sun_through(o + d * ground) * max(air.to_sun.y, 0.0) * air.scale.w / PI;
        light += result.through * floor * 0.5;
    }
    textureStore(sky_view_out, id.xy, vec4<f32>(light * air.scale.y, 1.0));
}

// Aerial perspective: per column of the view, march out through the air,
// the distances stretched by the aerial scale, storing at each slice what
// has been added and let through so far.
@compute @workgroup_size(8, 8, 1)
fn cs_aerial(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= AERIAL || id.y >= AERIAL {
        return;
    }
    let uv = (vec2<f32>(id.xy) + 0.5) / f32(AERIAL);
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let a = air.inverse_view_projection * vec4<f32>(ndc, 0.0, 1.0);
    let b = air.inverse_view_projection * vec4<f32>(ndc, 1.0, 1.0);
    let d = normalize(b.xyz / b.w - a.xyz / a.w);
    let o = vec3<f32>(0.0, PLANET + max(air.to_sun.w, 1.0), 0.0);
    let far = air.eye.w;
    var light = vec3<f32>(0.0);
    var through = vec3<f32>(1.0);
    var previous = 0.0;
    for (var z = 0u; z < AERIAL; z = z + 1u) {
        // Slices by the square: fine near, coarse far.
        let t = (f32(z) + 1.0) / f32(AERIAL);
        let distance = t * t * far;
        let start = o + d * previous * air.scale.z;
        let part = march(start, d, (distance - previous) * air.scale.z, 2);
        light += through * part.light;
        through *= part.through;
        let grey = dot(through, vec3<f32>(0.3333));
        textureStore(aerial_out, vec3<u32>(id.xy, z), vec4<f32>(light * air.scale.y, grey));
        previous = distance;
    }
}
