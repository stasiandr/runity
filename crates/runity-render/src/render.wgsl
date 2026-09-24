// The scene's lighting: one sun, hemisphere ambient, point and spot lights,
// distance fog, and the sky behind everything. Drawn in linear light into a
// high-dynamic-range buffer; post.wgsl turns that into a picture.

// Wind and what bends grass (foliage.rs): in the frame and in each shadow
// pass, so a thing and its shadow sway together.
struct Foliage {
    // level wind direction x and z, strength, time in seconds
    wind: vec4<f32>,
    // position and radius of each; radius 0 bends nothing
    benders: array<vec4<f32>, 8>,
    // the trample map's middle x and z, its size, 1 when there is one
    trample: vec4<f32>,
};

struct Frame {
    view_projection: mat4x4<f32>,
    // Direction the light travels: from the sun toward the ground.
    sun_direction: vec4<f32>,
    // Already multiplied by intensity on the CPU.
    sun_color: vec4<f32>,
    sky_color: vec4<f32>,
    ground_color: vec4<f32>,
    fog_color: vec4<f32>,
    // start, end, mode (0 linear, 1 exponential, 2 exponential squared),
    // density
    fog_range: vec4<f32>,
    camera_position: vec4<f32>,
    // Each cascade's: world to its map's clip space.
    light_view_projection: array<mat4x4<f32>, 4>,
    // depth bias, normal offset in world units, one texel in UV, how many
    // cascades there are (0: no shadows)
    shadow_params: vec4<f32>,
    // The camera view's third row: -dot(row, p) is how deep p is.
    view_depth: vec4<f32>,
    // Light cells across, down, deep; w how many lights there are.
    clusters: vec4<f32>,
    // near plane, ln(far / near), target width and height in pixels
    cluster_depth: vec4<f32>,
    // one texel of a lamp's shadow map, in UV
    light_shadow: vec4<f32>,
    inverse_view_projection: mat4x4<f32>,
    // Zenith colour; w is 1 for a procedural sky.
    sky_zenith: vec4<f32>,
    // Horizon colour; w is the cosine of the sun disc's radius.
    sky_horizon: vec4<f32>,
    // Below the horizon; w is the sky's exposure.
    sky_ground: vec4<f32>,
    // Each cascade's sphere: centre, and radius squared.
    cascade_spheres: array<vec4<f32>, 4>,
    // Each cascade's offset along the normal.
    cascade_bias: vec4<f32>,
    // Each cascade's depth bias, in its own map's depth.
    cascade_depth_bias: vec4<f32>,
    // 1 when there is ambient occlusion to read; the share of the direct
    // light it darkens too
    ambient_occlusion: vec4<f32>,
    // Hardware rays (ray.rs), 1 where asked and traced: sun shadows, lamp
    // shadows, occlusion; w the tangent of the sun disc's radius
    ray: vec4<f32>,
    // rays to the sun, occlusion rays, occlusion reach in metres
    ray_params: vec4<f32>,
    // reflection probes: how many, their last mip
    probe_params: vec4<f32>,
    // each probe: centre and blend distance; half size and 1 for box
    // projection
    probes: array<vec4<f32>, 16>,
    // each face's projection of a direction: +x, -x, +y, -y, +z, -z
    probe_faces: array<mat4x4<f32>, 6>,
    // volumetric fog: 1 when on, how far its cells reach, the near plane
    volume: vec4<f32>,
    // the air's colour, its density at the base height
    fog_medium: vec4<f32>,
    // base height, falloff per metre, anisotropy, the sky's share
    fog_shape: vec4<f32>,
    // the lamps' share
    fog_lamps: vec4<f32>,
    // behind everything, with a plain-colour sky; w: seconds the renderer
    // has run, for a material's shader to move by
    clear_color: vec4<f32>,
    foliage: Foliage,
    // the physical sky: 1 when on, the aerial grid's far end in metres
    air: vec4<f32>,
    // weather.rs: wetness, puddles, snow, rain; snowfall
    weather: array<vec4<f32>, 3>,
    // up to four water surfaces: height and 1 when there; the rectangle
    // it covers (min x, min z, max x, max z)
    waters: array<vec4<f32>, 8>,
    // clouds.rs: coverage, base, thickness, density; drift x and z, size,
    // how dark their shadows are
    clouds: array<vec4<f32>, 2>,
    // last frame's camera
    previous_view_projection: mat4x4<f32>,
    // screen-space reflections: 1 when on, how far, how thick, how many steps
    ssr: vec4<f32>,
    // dust in the air (volume.rs): centre and radius; colour and density.
    // How many is volume.w.
    puffs: array<vec4<f32>, 32>,
    // 1 when the clouds' pass marched dust devils or crest plumes
    dust: vec4<f32>,
    // how much it is night: the stars
    night: vec4<f32>,
    // the terrain drawn finely: the world into its own space, and back
    terrain_to_local: mat4x4<f32>,
    terrain_to_world: mat4x4<f32>,
    // its size, its cells, 1 when there is one, the finest spacing
    terrain: vec4<f32>,
    // its lowest and highest ground in the world; patches per ring side
    terrain_bounds: vec4<f32>,
    // what it is drawn with, for the mesh shader: an instance's numbers
    terrain_look: array<vec4<f32>, 7>,
    // glass by rays: 1 when on; its index of refraction; how many points
    // the lightning has, and its flash
    glass: vec4<f32>,
    // a stroke of lightning's channel: points, brightness in w
    bolt: array<vec4<f32>, 32>,
    // The irradiance volume (ddgi.rs): its first probe and spacing; its
    // probes along each axis and 1 when on; the biases along the normal
    // and toward the eye, and the farthest a distance counts.
    ddgi: array<vec4<f32>, 4>,
    // Virtual shadow maps (vsm.rs): the light's across and the pool's
    // pages across; its up and where depth starts; along it and how deep;
    // on, levels, a pixel's metres a metre (negative: anywhere), how far;
    // then each level's window, two levels a vec4.
    vsm: array<vec4<f32>, 8>,
    // ReSTIR (restir.rs): on, the reservoirs' size, the frame's number.
    restir: vec4<f32>,
    // the scene's distance field (distance.rs): its box and 1 when there
    // is one; its far corner and range in metres
    distance: array<vec4<f32>, 2>,
    // What a face turned sideways sees when the scene says its light from
    // all round (sky above, ground below); w is 1 then.
    ambient_equator: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_map: texture_depth_2d_array;
// A comparison sampler: the hardware does the depth test and the bilinear
// filter in one fetch, so every tap is already a 2x2 average.
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
// Ambient occlusion, a texel per pixel (ssao.wgsl).
@group(0) @binding(4) var occlusion: texture_2d<f32>;

// Point and spot lights, Forward+ (lights.rs): every light the view sees,
// and for each cell of a grid over the view — across, down and deep — the
// run of `light_indices` naming those that reach into it.
struct Light {
    // position, range
    position_range: vec4<f32>,
    // colour times intensity; w its first shadow map, or -1
    color_shadow: vec4<f32>,
    // for a spot, which way and the cosine of half its cone; -2 every way
    spot: vec4<f32>,
};
@group(0) @binding(6) var<storage, read> lights: array<Light>;
// Per cell: its lights' run in light_indices (start, count), then its
// decals' (start, count).
@group(0) @binding(7) var<storage, read> light_cells: array<vec4<u32>>;
@group(0) @binding(8) var<storage, read> light_indices: array<u32>;
// The lamps' shadow maps: a spot's one, a point's six cube faces.
@group(0) @binding(9) var light_shadow_map: texture_depth_2d_array;
@group(0) @binding(10) var<uniform> light_views: array<mat4x4<f32>, 24>;
// Reflection probes' pictures (reflections.rs): six layers a probe, a mip a
// step rougher.
@group(0) @binding(11) var probe_maps: texture_2d_array<f32>;
@group(0) @binding(12) var probe_sampler: sampler;

// Decals (decals.rs): pictures pressed down each box's -y onto what lies
// in it, listed by the same cells as the lights.
struct Decal {
    world_to_box: mat4x4<f32>,
    // linear colour, alpha
    color: vec4<f32>,
    // colour layer, normal layer (-1 none), normal scale, smoothness
    maps: vec4<f32>,
    axis_x: vec4<f32>,
    axis_up: vec4<f32>,
};
@group(0) @binding(13) var<storage, read> decals: array<Decal>;
@group(0) @binding(14) var decal_colours: texture_2d_array<f32>;
@group(0) @binding(15) var decal_normals: texture_2d_array<f32>;

// Volumetric fog (volume.rs): per cell of a grid over the view, the light
// the air between the eye and the cell adds (rgb) and lets through (a).
@group(0) @binding(16) var fog_volume: texture_3d<f32>;
@group(0) @binding(17) var fog_sampler: sampler;
// The compute passes that make it, in a group of their own.
@group(3) @binding(0) var fog_scatter_out: texture_storage_3d<rgba16float, write>;
@group(3) @binding(1) var fog_scatter_in: texture_3d<f32>;
@group(3) @binding(2) var fog_integrated_out: texture_storage_3d<rgba16float, write>;

// Smoke from a grid, sampled by the fog's cells (`volume::Smoke`).
struct SmokeBox {
    // low corner, 1 when there is smoke
    low: vec4<f32>,
    // high corner, extinction per unit of density
    high: vec4<f32>,
    // colour, glow
    color: vec4<f32>,
    // how much of the texture it fills
    fill: vec4<f32>,
};
@group(3) @binding(10) var smoke_volume: texture_3d<f32>;
@group(3) @binding(11) var smoke_sampler: sampler;
struct SmokeBoxes {
    boxes: array<SmokeBox, 4>,
    count: vec4<u32>,
};
@group(3) @binding(12) var<uniform> smokes: SmokeBoxes;

/// The smokes at a point: what they scatter (their colour times their
/// extinction), their extinction, and their glow.
struct SmokeHere {
    scatter: vec3<f32>,
    extinction: f32,
    glow: vec3<f32>,
};

fn smoke_at(p: vec3<f32>) -> SmokeHere {
    var here = SmokeHere(vec3<f32>(0.0), 0.0, vec3<f32>(0.0));
    for (var i = 0u; i < min(smokes.count.x, 4u); i = i + 1u) {
        let b = smokes.boxes[i];
        let t = (p - b.low.xyz) / max(b.high.xyz - b.low.xyz, vec3<f32>(1e-4));
        if any(t < vec3<f32>(0.0)) || any(t > vec3<f32>(1.0)) {
            continue;
        }
        // Its slab of the texture: the smokes lie one above another in z.
        let uvw = vec3<f32>(t.xy * b.fill.xy, (t.z * b.fill.z + b.fill.w) / 4.0);
        let s = textureSampleLevel(smoke_volume, smoke_sampler, uvw, 0.0);
        let density = s.r * 3.0 * b.high.w;
        let heat = s.g;
        here.scatter += b.color.rgb * density;
        here.extinction += density;
        // Fire glows by its heat, deep red to yellow-white.
        here.glow += mix(vec3<f32>(1.0, 0.18, 0.02), vec3<f32>(1.0, 0.75, 0.35), heat) * heat * heat * b.color.w * max(s.r * 3.0, 0.3);
    }
    return here;
}
const FOG_SIZE = vec3<u32>(160u, 90u, 64u);

// The physical sky (atmosphere.rs): the whole sky from the camera, and per
// cell of a grid over the view, what the air adds and lets through.
@group(0) @binding(18) var sky_view: texture_2d<f32>;
@group(0) @binding(19) var aerial: texture_3d<f32>;
// The solid scene's depth, from the prepass: what water sees under itself.
@group(0) @binding(20) var scene_depth: texture_depth_2d;
// The clouds (clouds.wgsl): what they add over the sky, and let through.
@group(0) @binding(21) var cloud_layer: texture_2d<f32>;
// The last frame, resolved: what screen-space reflections read.
@group(0) @binding(22) var last_frame: texture_2d<f32>;
// The drawn terrain's heights on its grid of cells (terrain.rs).
@group(0) @binding(23) var terrain_heights: texture_2d<f32>;
// How the grass is trampled round the camera (foliage.rs, TrampleMap):
// pressed, and the way out.
@group(0) @binding(30) var trample_map: texture_2d<f32>;
// The scene's signed distance field (distance.rs), 128 on the surface.
@group(0) @binding(31) var distance_field: texture_3d<f32>;

/// How far the nearest solid is from `p`, by the scene's distance field;
/// far away outside its box.
fn scene_distance(p: vec3<f32>) -> f32 {
    let low = frame.distance[0].xyz;
    let high = frame.distance[1].xyz;
    let uvw = (p - low) / max(high - low, vec3<f32>(1e-4));
    if any(uvw < vec3<f32>(0.0)) || any(uvw > vec3<f32>(1.0)) {
        return 1e4;
    }
    let v = textureSampleLevel(distance_field, fog_sampler, uvw, 0.0).r;
    return (v * 2.0 - 1.0) * frame.distance[1].w;
}

/// Occlusion by the distance field: stepping out along the normal, how
/// much nearer something is than the step went (Evans, "Fast Approximations
/// for Global Illumination on Dynamic Scenes").
fn field_occlusion(p: vec3<f32>, n: vec3<f32>) -> f32 {
    var occluded = 0.0;
    var weight = 1.0;
    for (var i = 1; i <= 5; i++) {
        let h = 0.12 * f32(i);
        occluded += (h - max(scene_distance(p + n * h), 0.0)) * weight;
        weight *= 0.6;
    }
    return clamp(1.0 - occluded * 2.2, 0.0, 1.0);
}

/// The share of the sun seen from `p` past what the field holds: marched
/// toward it, the nearest the ray passes to anything over how far it has
/// gone is how much of the disc shows (Quilez, soft shadows).
fn field_shadow(p: vec3<f32>, n: vec3<f32>, to_sun: vec3<f32>) -> f32 {
    var seen = 1.0;
    var t = 0.08;
    let start = p + n * 0.06;
    for (var i = 0; i < 40; i++) {
        let d = scene_distance(start + to_sun * t);
        if d > 1e3 {
            break;
        }
        seen = min(seen, 10.0 * d / t);
        if seen < 0.01 {
            return 0.0;
        }
        t += clamp(d, 0.04, 0.6);
        if t > 12.0 {
            break;
        }
    }
    return smoothstep(0.0, 1.0, seen);
}

/// Whether any of the dust wall can lie between the eye and a point: the
/// point is past where the ray enters the wall's side of its front (its
/// bulges included).
fn dust_can_reach(p: vec3<f32>) -> bool {
    let w = select(vec2<f32>(1.0, 0.0), normalize(frame.foliage.wind.xy), length(frame.foliage.wind.xy) > 1e-4);
    let eye = frame.camera_position.xyz;
    let front = -frame.weather[1].w + 260.0;
    if dot(eye.xz, w) < front {
        return true;
    }
    return dot(p.xz, w) < front;
}

/// How deep, along the view, the prepass's depth at a pixel is.
fn scene_view_depth(pixel: vec2<i32>) -> f32 {
    let d = textureLoad(scene_depth, pixel, 0);
    let near = frame.cluster_depth.x;
    let far = near * exp(frame.cluster_depth.y);
    return near * far / (far - d * (far - near));
}

/// How far behind what the prepass saw a point on a reflected ray is, in
/// metres along the view, and how deep it is; −2 when it is off the
/// screen or behind the eye.
fn ssr_behind(q: vec3<f32>) -> vec2<f32> {
    let clip = frame.view_projection * vec4<f32>(q, 1.0);
    if clip.w <= 0.0 {
        return vec2<f32>(-2.0, 0.0);
    }
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return vec2<f32>(-2.0, 0.0);
    }
    let seen = scene_view_depth(vec2<i32>(uv * frame.cluster_depth.zw));
    let ray = -dot(frame.view_depth, vec4<f32>(q, 1.0));
    return vec2<f32>(ray - seen, ray);
}

/// A reflection marched across the screen: from `p` along `r`, step by
/// step, until the ray passes just behind what the prepass saw there; the
/// colour is the last frame's at that place, moved to where it was then.
/// Its alpha is how much to trust it — none off the screen, less near its
/// edges, for rougher surfaces and far along the ray.
fn screen_reflection(p: vec3<f32>, r: vec3<f32>, roughness: f32) -> vec4<f32> {
    if frame.ssr.x < 0.5 || roughness > 0.6 {
        return vec4<f32>(0.0);
    }
    let steps = u32(frame.ssr.w);
    let far = frame.ssr.y;
    let thickness = frame.ssr.z;
    // Noise in where the steps fall, turned a little every frame, so bands
    // do not show: TAA averages it into a smooth reflection.
    let turn = frame.ambient_occlusion.w;
    let jitter = fract(pixel_noise(p.xz * 131.0 + p.y) + turn);
    var previous = 0.0;
    var in_front = true;
    for (var i = 1u; i <= steps; i = i + 1u) {
        // Steps closer near the surface, where hits are sharpest.
        let s = (f32(i) - jitter) / f32(steps);
        // Not closer than a hand's width: the surface's own depth is not a
        // hit, and depth is coarse far off.
        let t = max(far * s * s, 0.08);
        let found = ssr_behind(p + r * t);
        if found.x < -1.5 {
            break;
        }
        let slack = 0.02 + found.y * 0.004;
        // A crossing: in front at the last step, behind now — however far
        // behind, since a long step lands deep behind a thin thing it went
        // through. Starting out already behind something is being hidden.
        let crossed = in_front && found.x > slack;
        in_front = found.x <= slack;
        if crossed {
            // Halve back to where the ray first went behind.
            var lo = previous;
            var hi = t;
            var depth = found.x;
            for (var k = 0; k < 7; k = k + 1) {
                let mid = (lo + hi) * 0.5;
                let b = ssr_behind(p + r * mid);
                if b.x > 0.02 + b.y * 0.004 {
                    hi = mid;
                    depth = b.x;
                } else {
                    lo = mid;
                }
            }
            // Only now is it a hit: just behind the surface there, not
            // passing far behind something that stands in front.
            if depth < thickness + hi * 0.03 {
                let q = p + r * hi;
                let was = frame.previous_view_projection * vec4<f32>(q, 1.0);
                let then = was.xy / was.w;
                let at = vec2<f32>(then.x * 0.5 + 0.5, 0.5 - then.y * 0.5);
                let edge = min(min(at.x, 1.0 - at.x), min(at.y, 1.0 - at.y));
                let along = hi / far;
                let trust = smoothstep(0.0, 0.08, edge) * (1.0 - along) * (1.0 - smoothstep(0.2, 0.6, roughness));
                let color = textureSampleLevel(last_frame, fog_sampler, at, 0.0).rgb;
                return vec4<f32>(color, trust);
            }
        }
        previous = t;
    }
    return vec4<f32>(0.0);
}

/// Contact shadows: a short ray from `p` toward the sun, marched through
/// the depth of what is on the screen; 0 where something stands just in
/// its way — within a hand's thickness behind what the screen shows there —
/// fading back to 1 the further along it was met.
fn contact_shadow(p: vec3<f32>, n: vec3<f32>, to_sun: vec3<f32>, pixel: vec2<f32>) -> f32 {
    let reach = frame.ambient_occlusion.z;
    let steps = 12u;
    let turn = frame.ambient_occlusion.w;
    let jitter = fract(pixel_noise(pixel) + turn);
    let start = p + n * 0.015;
    for (var i = 0u; i < steps; i = i + 1u) {
        let along = (f32(i) + jitter) / f32(steps);
        let t = reach * along;
        let found = ssr_behind(start + to_sun * t);
        if found.x < -1.5 {
            break;
        }
        let slack = 0.01 + found.y * 0.002;
        if found.x > slack && found.x < 0.25 + t {
            return mix(0.0, 1.0, along * along);
        }
    }
    return 1.0;
}

fn cloud_hash(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q += dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

fn cloud_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(cloud_hash(i), cloud_hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x),
        mix(cloud_hash(i + vec3<f32>(0.0, 1.0, 0.0)), cloud_hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(cloud_hash(i + vec3<f32>(0.0, 0.0, 1.0)), cloud_hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x),
        mix(cloud_hash(i + vec3<f32>(0.0, 1.0, 1.0)), cloud_hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}

/// How much of the sun a cloud above takes from a point: the same density
/// the cloud pass marches, looked up once where the way to the sun crosses
/// the middle of the layer.
fn cloud_shadow(p: vec3<f32>) -> f32 {
    let shape = frame.clouds[0];
    let drift = frame.clouds[1];
    if shape.x <= 0.0 || drift.w <= 0.0 {
        return 1.0;
    }
    let to_sun = -normalize(frame.sun_direction.xyz);
    if to_sun.y < 0.05 {
        return 1.0;
    }
    let middle = shape.y + shape.z * 0.4;
    let q_world = p + to_sun * ((middle - p.y) / to_sun.y);
    let drifted = q_world - vec3<f32>(drift.x, 0.0, drift.y) * frame.foliage.wind.w;
    let q = drifted / drift.z;
    let noise = cloud_noise(q) * 0.55 + cloud_noise(q * 2.03) * 0.28 + cloud_noise(q * 4.1) * 0.17;
    // The profile at 0.4 of the way up is nearly whole.
    let d = clamp((noise * 0.95 - (1.0 - shape.x)) / max(shape.x, 0.05), 0.0, 1.0) * shape.w;
    return 1.0 - drift.w * (1.0 - exp(-d * 4.0));
}

/// The physical sky the way `direction` looks: its table's azimuth
/// across, latitude up, squeezed towards the horizon as it was filled.
fn physical_sky(direction: vec3<f32>) -> vec3<f32> {
    var d = normalize(direction);
    // Below the horizon, the horizon a little dimmed: the table's ground is
    // a planet's, far below any scene's.
    var below = 1.0;
    if d.y < 0.0 {
        below = mix(1.0, 0.55, smoothstep(0.0, 0.4, -d.y));
        d = normalize(vec3<f32>(d.x, 0.0, d.z) + vec3<f32>(0.0, 0.01, 0.0));
    }
    let azimuth = atan2(d.z, d.x);
    let latitude = asin(clamp(d.y, -1.0, 1.0));
    let t = sign(latitude) * sqrt(abs(latitude) / 1.5707963);
    let uv = vec2<f32>(azimuth / 6.2831853 + 0.5, t * 0.5 + 0.5);
    return textureSampleLevel(sky_view, fog_sampler, uv, 0.0).rgb * below;
}

/// The sky's light on a face turned `n`, from the physical sky itself: its
/// picture averaged over the half of it the face sees, as a share of what
/// a face turned straight up sees — so the sky's light keeps its level
/// and gains its direction: warmer and brighter on the side toward the
/// sun, bluer away from it. What lets relief in shade still read.
fn sky_toward(n: vec3<f32>) -> vec3<f32> {
    let t = basis_of(n);
    var sum = physical_sky(n) * 2.0;
    for (var i = 0; i < 6; i = i + 1) {
        let a = f32(i) * 1.0471976;
        let d = normalize(n + (t.t * cos(a) + t.b * sin(a)) * 1.2);
        sum += physical_sky(d);
    }
    return sum / 8.0;
}

/// The distance fog's colour: the scene's, or with a physical sky the sky
/// itself just above the horizon that way, so the distance melts into it.
fn fog_color_towards(direction: vec3<f32>) -> vec3<f32> {
    if frame.air.x < 0.5 {
        return frame.fog_color.rgb;
    }
    let level = vec3<f32>(direction.x, max(direction.y, 0.03), direction.z);
    return physical_sky(level) * frame.sky_ground.w;
}

/// A colour seen through the air between it and the eye: the physical
/// sky's aerial perspective.
fn through_air(color: vec3<f32>, pixel: vec2<f32>, distance: f32) -> vec3<f32> {
    if frame.air.x < 0.5 {
        return color;
    }
    let uv = pixel / frame.cluster_depth.zw;
    // Slices by the square root of distance, as they were filled; each
    // holds the sum to its far end.
    let t = sqrt(clamp(distance / frame.air.y, 0.0, 1.0));
    let w = clamp(t - 0.5 / 32.0, 0.0, 1.0);
    let a = textureSampleLevel(aerial, fog_sampler, vec3<f32>(uv, w), 0.0);
    return color * a.a + a.rgb;
}

// The shadow pass's one matrix: the cascade being drawn. Beside the frame
// at binding 3, in the shadow pass's own group.
struct Caster {
    view_projection: mat4x4<f32>,
    foliage: Foliage,
};
@group(0) @binding(3) var<uniform> caster: Caster;

// maps: begin
// The surface's own image. Every draw binds one; an untextured material
// binds a single white pixel, so the shader never needs a branch and an
// untextured surface is its colour times one.
@group(1) @binding(0) var surface_texture: texture_2d<f32>;
@group(1) @binding(1) var surface_sampler: sampler;
// URP Lit's other maps: normal (tangent space), mask (r metallic, g
// occlusion, a smoothness) and emission. A surface without one binds a
// neutral one: a flat normal, white.
@group(1) @binding(2) var normal_map: texture_2d<f32>;
@group(1) @binding(3) var mask_map: texture_2d<f32>;
@group(1) @binding(4) var emission_map: texture_2d<f32>;

// The maps read, by the draw's bound ones; `maps` (the instance's
// handles) is for the bindless shader in their place (bindless.rs).
fn surface_at(maps: vec4<u32>, uv: vec2<f32>) -> vec4<f32> {
    return textureSample(surface_texture, surface_sampler, uv);
}
fn normal_at(maps: vec4<u32>, uv: vec2<f32>) -> vec4<f32> {
    return textureSample(normal_map, surface_sampler, uv);
}
fn mask_at(maps: vec4<u32>, uv: vec2<f32>) -> vec4<f32> {
    return textureSample(mask_map, surface_sampler, uv);
}
fn emission_at(maps: vec4<u32>, uv: vec2<f32>) -> vec4<f32> {
    return textureSample(emission_map, surface_sampler, uv);
}
// The textures a material hands its own shader, in the order the shader's
// `// runity:textures` line names them; white where it has none.
@group(1) @binding(5) var material_texture_0: texture_2d<f32>;
@group(1) @binding(6) var material_texture_1: texture_2d<f32>;
@group(1) @binding(7) var material_texture_2: texture_2d<f32>;
@group(1) @binding(8) var material_texture_3: texture_2d<f32>;

// A material's own texture `slot` (0 to 3) at `uv`, for its `surface`:
// repeated past the edges and mipmapped as the base map is. By the
// gradients of `uv` taken here, so it may be read inside a branch; it
// has to be called where every pixel of the draw still runs together,
// as `textureSample` does. A slot past the fourth is white.
fn texture_at(in: SurfaceIn, slot: u32, uv: vec2<f32>) -> vec4<f32> {
    let dx = dpdx(uv);
    let dy = dpdy(uv);
    switch slot {
        case 0u: { return textureSampleGrad(material_texture_0, surface_sampler, uv, dx, dy); }
        case 1u: { return textureSampleGrad(material_texture_1, surface_sampler, uv, dx, dy); }
        case 2u: { return textureSampleGrad(material_texture_2, surface_sampler, uv, dx, dy); }
        case 3u: { return textureSampleGrad(material_texture_3, surface_sampler, uv, dx, dy); }
        default: { return vec4<f32>(1.0); }
    }
}
// maps: end

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) model_0: vec4<f32>,
    @location(4) model_1: vec4<f32>,
    @location(5) model_2: vec4<f32>,
    @location(6) model_3: vec4<f32>,
    @location(7) color_and_shading: vec4<f32>,
    // metallic, smoothness, alpha, alpha-clip threshold
    @location(10) surface: vec4<f32>,
    // emission rgb; w packs the switches: 1 highlights, 2 reflections,
    // 4 receives shadows, 8 premultiplied
    @location(11) emission: vec4<f32>,
    // tiling xy, offset zw
    @location(12) uv_transform: vec4<f32>,
    // normal scale, occlusion strength
    @location(13) detail: vec4<f32>,
    // the material's own numbers, for its shader: in.params in `surface`
    @location(14) params_0: vec4<f32>,
    @location(15) params_1: vec4<f32>,
    // light under the surface: its colour, and how far it goes
    @location(16) subsurface: vec4<f32>,
    // the maps' handles, two to a number (bindless.rs): base, normal,
    // mask, emission in the low halves, the material's own four
    // textures in the high
    @location(17) maps: vec4<u32>,
};

struct VertexOutput {
    // The same depth in the prepass and the scene's pass, which starts
    // from the prepass's.
    @invariant @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) base_color: vec3<f32>,
    // 0 lit, 1 unlit, 2 lit with the metre grid.
    @location(3) shading: f32,
    @location(4) uv: vec2<f32>,
    @location(5) surface: vec4<f32>,
    @location(6) emission: vec4<f32>,
    @location(7) detail: vec4<f32>,
    @location(8) params_0: vec4<f32>,
    @location(9) params_1: vec4<f32>,
    @location(10) subsurface: vec4<f32>,
    @location(11) @interpolate(flat) maps: vec4<u32>,
};

// One pose's skinning matrices. Bound per draw with a dynamic offset, so
// two characters in different poses cost two offsets rather than two
// pipelines.
struct Pose {
    joints: array<mat4x4<f32>, 64>,
};
@group(2) @binding(0) var<uniform> pose: Pose;

struct SkinInput {
    @location(8) joints: vec4<u32>,
    @location(9) weights: vec4<f32>,
};

/// The skinned vertex stage.
///
/// The weighted sum of matrices is taken first and applied once, rather than
/// transforming the vertex by each joint and averaging the results. The two
/// agree for rigid motion and differ under scale, and the first is both
/// cheaper and what every exporter assumes.
@vertex
fn vs_skinned(in: VertexInput, skin: SkinInput) -> VertexOutput {
    var skinning =
        pose.joints[skin.joints.x] * skin.weights.x +
        pose.joints[skin.joints.y] * skin.weights.y +
        pose.joints[skin.joints.z] * skin.weights.z +
        pose.joints[skin.joints.w] * skin.weights.w;

    // Weights that sum to nothing would collapse the vertex onto the origin.
    // An identity keeps it where the artist put it.
    let total = skin.weights.x + skin.weights.y + skin.weights.z + skin.weights.w;
    if total < 0.0001 {
        skinning = mat4x4<f32>(
            vec4<f32>(1.0, 0.0, 0.0, 0.0),
            vec4<f32>(0.0, 1.0, 0.0, 0.0),
            vec4<f32>(0.0, 0.0, 1.0, 0.0),
            vec4<f32>(0.0, 0.0, 0.0, 1.0),
        );
    }

    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    let posed = skinning * vec4<f32>(in.position, 1.0);
    let world = model * posed;

    var out: VertexOutput;
    out.clip_position = frame.view_projection * world;
    out.world_position = world.xyz;
    out.normal = (model * (skinning * vec4<f32>(in.normal, 0.0))).xyz;
    out.base_color = in.color_and_shading.rgb;
    out.shading = in.color_and_shading.w;
    out.uv = in.uv * in.uv_transform.xy + in.uv_transform.zw;
    out.surface = in.surface;
    out.emission = in.emission;
    out.detail = in.detail;
    out.params_0 = in.params_0;
    out.params_1 = in.params_1;
    out.subsurface = in.subsurface;
    out.maps = in.maps;
    return out;
}

/// Where a vertex ends up in the wind and round what bends grass.
///
/// A thing bends away from the wind by its height above its own origin —
/// a trunk's foot stays put, its crown moves — rocking in gusts
/// that roll across the ground, with a quick flutter on top. `amount` is
/// the material's `wind`; zero moves nothing.
fn swayed(world: vec3<f32>, origin: vec3<f32>, amount: f32, f: Foliage) -> vec3<f32> {
    if amount <= 0.0 {
        return world;
    }
    let height = max(world.y - origin.y, 0.0);
    let direction = vec3<f32>(f.wind.x, 0.0, f.wind.y);
    let strength = f.wind.z;
    let t = f.wind.w;
    // Gusts travel downwind: the phase is where the thing stands along it.
    let along = dot(origin.xz, f.wind.xy);
    let gust = 0.55 + 0.3 * sin(t * 0.9 - along * 0.35) + 0.15 * sin(t * 2.3 - along * 0.9 + origin.x);
    // Curved up to a metre — a blade of grass bows — and straight above,
    // so a tall tree's crown sways a hand's width, not a metre.
    let bend = amount * strength * height * min(height, 1.0) * 0.04 * gust;
    let flutter = amount * strength * min(height, 1.0) * 0.015
        * sin(t * 7.0 + dot(world, vec3<f32>(1.7, 2.3, 1.1)));
    var moved = world + direction * bend + vec3<f32>(-direction.z, 0.3, direction.x) * flutter;
    // Branches: how far out from the trunk's axis a point is. Each branch —
    // a band round the trunk and up it — bobs and swings on its own phase,
    // more the further out; leaves at the tips flutter fast.
    let out = world.xz - origin.xz;
    let reach = length(out);
    if reach > 0.3 && height > 0.5 {
        let around = atan2(out.y, out.x);
        let branch = floor(around * 1.3) * 3.7 + floor(height * 1.5) * 1.9;
        let swing = sin(t * 1.9 + branch + along * 0.2) * (0.6 + 0.4 * gust);
        let sway = amount * strength * reach * 0.02 * swing;
        let side = vec3<f32>(-direction.z, 0.0, direction.x);
        moved += side * sway + vec3<f32>(0.0, sway * 0.6, 0.0) + direction * abs(sway) * 0.5;
        let leaf = amount * strength * min(reach, 3.0) * 0.006
            * sin(t * 11.0 + dot(world, vec3<f32>(3.1, 1.3, 2.7)) + branch);
        moved += vec3<f32>(leaf, leaf * 0.7, -leaf);
    }
    // Kept roughly its length: what leans over also drops.
    moved.y -= bend * bend / max(2.0 * height, 0.2);

    // Pushed out of the way, and down, by what walks through it.
    for (var i = 0u; i < 8u; i = i + 1u) {
        let b = f.benders[i];
        if b.w <= 0.0 {
            continue;
        }
        let away = moved.xz - b.xz;
        let distance = length(away);
        let push = (1.0 - smoothstep(0.0, b.w, distance)) * min(height, 1.0) * amount;
        if push > 0.0 {
            let out = away / max(distance, 1e-3);
            moved = moved + vec3<f32>(out.x, 0.0, out.y) * push * 0.5;
            moved.y -= push * height * 0.6;
        }
    }
    return moved;
}

/// A vertex of grass pressed down and out by what has walked through it
/// and not yet sprung back: the trample map.
fn trampled(world: vec3<f32>, origin: vec3<f32>, amount: f32, f: Foliage) -> vec3<f32> {
    if amount <= 0.0 || f.trample.w < 0.5 {
        return world;
    }
    let size = f.trample.z;
    let t = (world.xz - f.trample.xy) / size + 0.5;
    if any(t < vec2<f32>(0.0)) || any(t >= vec2<f32>(1.0)) {
        return world;
    }
    let cells = vec2<f32>(textureDimensions(trample_map));
    let texel = textureLoad(trample_map, vec2<i32>(t * cells), 0);
    let pressed = texel.r * min(amount, 1.0);
    if pressed <= 0.0 {
        return world;
    }
    let height = max(world.y - origin.y, 0.0);
    let away = texel.gb * 2.0 - 1.0;
    // Laid over, not squashed: out along the way it was pushed, and down
    // as far as it leans.
    var moved = world + vec3<f32>(away.x, 0.0, away.y) * pressed * min(height, 1.0) * 0.9;
    moved.y -= pressed * height * 0.8;
    return moved;
}

/// The depth-only pass, seen from the sun, one cascade at a time.
@vertex
fn vs_shadow(in: VertexInput) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    let world = swayed((model * vec4<f32>(in.position, 1.0)).xyz, in.model_3.xyz, in.detail.z, caster.foliage);
    return caster.view_projection * vec4<f32>(world, 1.0);
}

struct ClipOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // alpha, threshold
    @location(1) alpha: vec2<f32>,
    @location(2) @interpolate(flat) maps: vec4<u32>,
};

/// The same, for what is cut out by its alpha: a leaf's shadow is a leaf.
@vertex
fn vs_shadow_clip(in: VertexInput) -> ClipOut {
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    var out: ClipOut;
    let world = swayed((model * vec4<f32>(in.position, 1.0)).xyz, in.model_3.xyz, in.detail.z, caster.foliage);
    out.position = caster.view_projection * vec4<f32>(world, 1.0);
    out.uv = in.uv * in.uv_transform.xy + in.uv_transform.zw;
    out.alpha = in.surface.zw;
    out.maps = in.maps;
    return out;
}

@fragment
fn fs_shadow_clip(in: ClipOut) {
    if in.alpha.x * surface_at(in.maps, in.uv).a < in.alpha.y {
        discard;
    }
}

// ray: stub begin
// On a device that does not trace, nothing is in the way of any ray; the
// frame never asks one there. ray.rs puts ray.wgsl in place of this.
fn ray_clear(origin: vec3<f32>, direction: vec3<f32>, start: f32, reach: f32, mask: u32) -> f32 {
    return 1.0;
}

fn ray_reflection(origin: vec3<f32>, direction: vec3<f32>) -> vec4<f32> {
    return vec4<f32>(0.0);
}

fn ray_refraction(position: vec3<f32>, incoming: vec3<f32>, normal: vec3<f32>, ior: f32) -> vec4<f32> {
    return vec4<f32>(0.0);
}
// ray: stub end

/// What rays can be asked to see: everything drawn but the terrain, and
/// the terrain (ray.rs gives each instance one of these).
const RAY_THINGS: u32 = 1u;
const RAY_TERRAIN: u32 = 2u;
const RAY_GLASS: u32 = 4u;
/// How far a ray goes before the terrain can stop it. The rays see the
/// terrain as its heightfield's triangles, a metre and more apart; what is
/// drawn is finer — folded down to centimetres, rippled — and dips under
/// them, so a ray from the sand would hit the coarse sand over it and
/// crack every ripple with shadow. Past this the ray has cleared the
/// ripples, and a dune still shadows the next.
const RAY_TERRAIN_GAP: f32 = 1.5;

/// Nothing within `reach` along `direction`: nothing drawn, and no terrain
/// past the gap.
fn ray_visible(origin: vec3<f32>, direction: vec3<f32>, reach: f32) -> f32 {
    let things = ray_clear(origin, direction, 0.0, reach, RAY_THINGS);
    if things < 0.5 || reach <= RAY_TERRAIN_GAP {
        return things;
    }
    return ray_clear(origin, direction, RAY_TERRAIN_GAP, reach, RAY_TERRAIN);
}

/// Noise that differs pixel to pixel and hides its pattern well (Jimenez's
/// interleaved gradient noise): what turns each pixel's handful of rays.
fn pixel_noise(pixel: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

struct Basis {
    t: vec3<f32>,
    b: vec3<f32>,
};

/// Two directions square to `n` and to each other.
fn basis_of(n: vec3<f32>) -> Basis {
    let other = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.9);
    let t = normalize(cross(other, n));
    return Basis(t, cross(n, t));
}

/// The sun by rays: a few towards points across its disc, so the shadow
/// sharpens where it touches its caster and softens away from it.
fn traced_sun(position: vec3<f32>, normal: vec3<f32>, to_sun: vec3<f32>, pixel: vec2<f32>) -> f32 {
    let count = max(u32(frame.ray_params.x), 1u);
    let spread = frame.ray.w;
    let frame_of = basis_of(to_sun);
    let origin = position + normal * (0.01 + 0.001 * length(position - frame.camera_position.xyz));
    let turn = pixel_noise(pixel) * 6.2831853;
    var lit = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let r = sqrt((f32(i) + 0.5) / f32(count)) * spread;
        let a = turn + f32(i) * 2.3999632;
        let d = normalize(to_sun + (frame_of.t * cos(a) + frame_of.b * sin(a)) * r);
        lit += ray_clear(origin, d, 0.0, 1.0e4, RAY_THINGS);
    }
    // The terrain by one ray, past the gap: a dune's shadow on the next is
    // broad, and the frames' jitter softens its edge.
    let terrain = ray_clear(origin, normalize(to_sun + (frame_of.t * cos(turn) + frame_of.b * sin(turn)) * spread * 0.7), RAY_TERRAIN_GAP, 1.0e4, RAY_TERRAIN);
    return lit / f32(count) * terrain;
}

/// Occlusion by rays: short ones over the hemisphere, cosine-weighted; the
/// share that reach `reach` without hitting anything.
fn traced_occlusion(position: vec3<f32>, normal: vec3<f32>, pixel: vec2<f32>) -> f32 {
    let count = max(u32(frame.ray_params.y), 1u);
    let reach = frame.ray_params.z;
    let frame_of = basis_of(normal);
    let origin = position + normal * 0.01;
    let n1 = pixel_noise(pixel);
    let n2 = pixel_noise(pixel.yx + vec2<f32>(37.0, 11.0));
    var open = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let u = fract((f32(i) + n1) / f32(count));
        let phi = fract(f32(i) * 0.618034 + n2) * 6.2831853;
        let r = sqrt(u);
        let d = frame_of.t * (r * cos(phi)) + frame_of.b * (r * sin(phi)) + normal * sqrt(1.0 - u);
        // Only what is drawn: the terrain's coarse triangles would shut in
        // every ripple, and the ground under a thing is too close to it for
        // the gap.
        open += ray_clear(origin, d, 0.0, reach, RAY_THINGS);
    }
    return open / f32(count);
}

/// How much sun reaches a point: 1.0 in the open, 0.0 in full shadow.
///
/// From the first cascade whose sphere holds the point — the finest one
/// that covers it — and fading out over the last tenth of the last one, so
/// the shadow distance is not a line on the ground.
// ReSTIR's reservoirs, one a pixel: the lamp chosen, as bits; the weights'
// sum; how many it stands for; its weight, its shadow in it.
@group(0) @binding(27) var<storage, read> restir_shade: array<vec4<f32>>;

// Virtual shadow maps (vsm.rs): each level's window of pages, the pool's
// tile a drawn page is in, plus one; 0 for none.
@group(0) @binding(26) var<storage, read> vsm_pages: array<u32>;

const VSM_PAGE: f32 = 128.0;
const VSM_FINEST: f32 = 0.015;
const VSM_WINDOW: i32 = 32;
/// The pool is the layer of the shadow map past the cascades.
const VSM_LAYER: i32 = 4;

struct VsmTexel {
    uv: vec2<f32>,
    // the tile's texels, less half a texel all round: where taps may go
    low: vec2<f32>,
    high: vec2<f32>,
    depth: f32,
    texel: f32,
    found: bool,
};

/// Where `p` (turned `normal`) is in the virtual shadow map: the finest
/// drawn page for its distance, or a coarser one while that is not drawn.
fn vsm_find(p: vec3<f32>, normal: vec3<f32>, push: f32) -> VsmTexel {
    var out: VsmTexel;
    out.found = false;
    let distance_to = distance(p, frame.camera_position.xyz);
    if distance_to > frame.vsm[3].w {
        return out;
    }
    let footprint = frame.vsm[3].z;
    let pixel = select(distance_to * footprint, -footprint, footprint < 0.0);
    let levels = u32(frame.vsm[3].y);
    let pool = frame.vsm[0].w;
    var level = u32(clamp(ceil(log2(max(pixel, 1e-9) / VSM_FINEST)), 0.0, f32(levels) - 1.0));
    for (; level < levels; level = level + 1u) {
        let texel = VSM_FINEST * f32(1u << level);
        let size = texel * VSM_PAGE;
        let q = p + normal * texel * push;
        let u = dot(q, frame.vsm[0].xyz) / size;
        let v = dot(q, frame.vsm[1].xyz) / size;
        let page = vec2<i32>(i32(floor(u)), i32(floor(v)));
        let windows = frame.vsm[4u + level / 2u];
        let window = select(windows.xy, windows.zw, (level & 1u) == 1u);
        let slot = page - vec2<i32>(window);
        if any(slot < vec2<i32>(0)) || any(slot >= vec2<i32>(VSM_WINDOW)) {
            continue;
        }
        let entry = vsm_pages[level * u32(VSM_WINDOW * VSM_WINDOW) + u32(slot.y * VSM_WINDOW + slot.x)];
        if entry == 0u {
            continue;
        }
        let tile = vec2<f32>(f32((entry - 1u) % u32(pool)), f32((entry - 1u) / u32(pool)));
        let local = vec2<f32>(u - f32(page.x), f32(page.y + 1) - v);
        let side = pool * VSM_PAGE;
        out.uv = (tile + local) / pool;
        out.low = (tile * VSM_PAGE + 0.5) / side;
        out.high = ((tile + 1.0) * VSM_PAGE - 0.5) / side;
        out.depth = (dot(q, frame.vsm[2].xyz) - frame.vsm[1].w) / frame.vsm[2].w;
        out.texel = 1.0 / side;
        out.found = true;
        return out;
    }
    return out;
}

/// The sun through the virtual shadow map: nine taps inside the page.
fn vsm_sunlight(p: vec3<f32>, normal: vec3<f32>) -> f32 {
    let at = vsm_find(p, normal, 1.5);
    if !at.found || at.depth > 1.0 || at.depth < 0.0 {
        return 1.0;
    }
    // A centimetre, in the pages' depth.
    let reference = at.depth - 0.01 / frame.vsm[2].w;
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let tap = clamp(at.uv + vec2<f32>(f32(x), f32(y)) * at.texel, at.low, at.high);
            sum = sum + textureSampleCompareLevel(shadow_map, shadow_sampler, tap, VSM_LAYER, reference);
        }
    }
    let fade = clamp((frame.vsm[3].w - distance(p, frame.camera_position.xyz)) / (frame.vsm[3].w * 0.1), 0.0, 1.0);
    return mix(1.0, sum / 9.0, fade);
}

fn sunlight(world_position: vec3<f32>, normal: vec3<f32>) -> f32 {
    if frame.vsm[3].x > 0.5 {
        return vsm_sunlight(world_position, normal);
    }
    let count = u32(frame.shadow_params.w + 0.5);
    if count == 0u {
        return 1.0;
    }
    var cascade = count;
    for (var i = 0u; i < count; i = i + 1u) {
        let d = world_position - frame.cascade_spheres[i].xyz;
        if dot(d, d) < frame.cascade_spheres[i].w {
            cascade = i;
            break;
        }
    }
    if cascade == count {
        // Past the shadow distance: lit, not shadowed. The opposite makes
        // everything beyond it a wall of darkness.
        return 1.0;
    }

    // Offsetting along the normal before the lookup is what handles grazing
    // angles: there the depth error grows with the slope, and no constant
    // bias large enough to cover it is small enough to keep contact.
    let offset = world_position + normal * frame.cascade_bias[cascade];
    let light_clip = frame.light_view_projection[cascade] * vec4<f32>(offset, 1.0);
    let ndc = light_clip.xyz / light_clip.w;

    // Clip space is -1..1 across and 0..1 deep; the map is indexed 0..1 with
    // v running the other way.
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if ndc.z > 1.0 || uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return 1.0;
    }

    let reference = ndc.z - frame.cascade_depth_bias[cascade];
    let texel = frame.shadow_params.z;
    // Nine taps, each of them already a hardware 2x2, so the edge is soft
    // enough that the map's resolution stops being visible as stairs.
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let tap = uv + vec2<f32>(f32(x), f32(y)) * texel;
            sum = sum + textureSampleCompareLevel(shadow_map, shadow_sampler, tap, i32(cascade), reference);
        }
    }
    let lit = sum / 9.0;

    // The last cascade fades to lit over its outer tenth.
    let last = frame.cascade_spheres[count - 1u];
    let from_centre = length(world_position - last.xyz);
    let radius = sqrt(last.w);
    let fade = clamp((radius - from_centre) / (radius * 0.1), 0.0, 1.0);
    return mix(1.0, lit, fade);
}

/// How much stands between a point and the sun, metres: from the shadow
/// map, how far past the first surface toward the sun the point lies — the
/// thickness of what it is under. −1 when there is no map there.
fn sun_thickness(world_position: vec3<f32>, normal: vec3<f32>) -> f32 {
    if frame.vsm[3].x > 0.5 {
        let at = vsm_find(world_position, normal, -1.5);
        if !at.found || at.depth > 1.0 {
            return -1.0;
        }
        let size = vec2<f32>(textureDimensions(shadow_map));
        let first = textureLoad(shadow_map, vec2<i32>(at.uv * size), VSM_LAYER, 0);
        return max(at.depth - first, 0.0) * frame.vsm[2].w;
    }
    let count = u32(frame.shadow_params.w + 0.5);
    if count == 0u {
        return -1.0;
    }
    var cascade = count;
    for (var i = 0u; i < count; i = i + 1u) {
        let d = world_position - frame.cascade_spheres[i].xyz;
        if dot(d, d) < frame.cascade_spheres[i].w {
            cascade = i;
            break;
        }
    }
    if cascade == count {
        return -1.0;
    }
    // Pushed in a little, against the normal: the point is under its own
    // surface, not on it.
    let inside = world_position - normal * frame.cascade_bias[cascade];
    let light = frame.light_view_projection[cascade];
    let clip = light * vec4<f32>(inside, 1.0);
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if ndc.z > 1.0 || uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return -1.0;
    }
    let size = vec2<f32>(textureDimensions(shadow_map));
    let first = textureLoad(shadow_map, vec2<i32>(uv * size), i32(cascade), 0);
    // An orthographic light: depth is metres times its scale along the view.
    let per_metre = max(abs(light[2][2]), 1e-6);
    return max(ndc.z - first, 0.0) / per_metre;
}

/// The light cell a fragment is in.
fn light_cell(pixel: vec2<f32>, world_position: vec3<f32>) -> u32 {
    let tiles = vec2<u32>(frame.clusters.xy);
    let slices = u32(frame.clusters.z);
    let across = min(vec2<u32>(pixel / frame.cluster_depth.zw * frame.clusters.xy), tiles - vec2<u32>(1u));
    let depth = -dot(frame.view_depth, vec4<f32>(world_position, 1.0));
    let t = log(max(depth, frame.cluster_depth.x) / frame.cluster_depth.x) / frame.cluster_depth.y;
    let slice = min(u32(max(t, 0.0) * f32(slices)), slices - 1u);
    return (slice * tiles.y + across.y) * tiles.x + across.x;
}

/// How much of a lamp reaches a point past what stands between them: its
/// shadow map, a spot's one or the cube face of a point's six that looks
/// the point's way. 1 for a lamp with none.
fn lamp_shadow(light: Light, position: vec3<f32>, normal: vec3<f32>, distance_to: f32) -> f32 {
    let first = light.color_shadow.w;
    if first < 0.0 {
        return 1.0;
    }
    var layer = u32(first + 0.5);
    let point = light.spot.w < -1.5;
    let d = position - light.position_range.xyz;
    if point {
        let a = abs(d);
        if a.x >= a.y && a.x >= a.z {
            layer += select(1u, 0u, d.x > 0.0);
        } else if a.y >= a.z {
            layer += select(3u, 2u, d.y > 0.0);
        } else {
            layer += select(5u, 4u, d.z > 0.0);
        }
    }
    // How big one texel is where the point is: the map spreads over the
    // cone, wider the farther from the lamp. Both offsets clear it.
    let cos_half = select(light.spot.w, 0.7071, point);
    let tan_half = sqrt(max(1.0 - cos_half * cos_half, 1e-4)) / max(cos_half, 1e-3);
    let texel = 2.0 * distance_to * tan_half * frame.light_shadow.x;
    let offset = position + normal * texel * 1.5;
    let clip = light_views[layer] * vec4<f32>(offset, 1.0);
    if clip.w <= 0.0 {
        return 1.0;
    }
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return 1.0;
    }
    // Compared a little nearer the lamp, in metres: its depth is a
    // perspective one, so the bias is put back through the same curve.
    let near = 0.05;
    let far = light.position_range.w;
    let z = max(clip.w - texel - 0.02, near);
    let reference = far / (far - near) * (1.0 - near / z);
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let tap = uv + vec2<f32>(f32(x), f32(y)) * frame.light_shadow.x;
            sum = sum + textureSampleCompareLevel(light_shadow_map, shadow_sampler, tap, i32(layer), reference);
        }
    }
    return sum / 9.0;
}

/// How deep, along the view, the fog grid's `t` (0 at the near plane, 1 at
/// its far end) is: thin slices near, thick far.
fn fog_depth(t: f32) -> f32 {
    let near = frame.volume.z;
    return near * pow(frame.volume.y / near, t);
}

/// The air's density at a point: thickest at the base height, thinning
/// above it.
fn fog_density(p: vec3<f32>) -> f32 {
    let above = max(p.y - frame.fog_shape.x, 0.0);
    var density = frame.fog_medium.w * exp(-frame.fog_shape.y * above);
    // A sandstorm rolls: its sand comes in billows driven by the wind.
    let storm = frame.weather[1].y;
    if storm > 0.0 {
        let wind = vec3<f32>(frame.foliage.wind.x, 0.0, frame.foliage.wind.y) * max(frame.foliage.wind.z, 0.5) * 6.0;
        let q = (p - wind * frame.foliage.wind.w) * 0.06;
        let billow = cloud_noise(q) * 0.6 + cloud_noise(q * 2.3 + 5.0) * 0.4;
        density *= mix(1.0, 0.25 + billow * 1.6, storm);
    }
    return density;
}

/// The dust kicked up in the air at a point: its colour times its
/// density, and its density. Each puff soft to its edge and lumpy inside.
fn puff_dust(p: vec3<f32>) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    let count = u32(frame.volume.w);
    for (var i = 0u; i < count; i = i + 1u) {
        let at = frame.puffs[2u * i];
        let r = length(p - at.xyz) / at.w;
        if r >= 1.0 {
            continue;
        }
        let edge = (1.0 - r * r) * (1.0 - r * r);
        let lumps = 0.45 + 1.1 * cloud_noise(p * 7.0 + at.xyz * 3.1);
        let look = frame.puffs[2u * i + 1u];
        let d = look.w * edge * lumps;
        sum += vec4<f32>(look.rgb * d, d);
    }
    return sum;
}

/// Henyey–Greenstein: how much light turning by an angle of this cosine
/// the air throws, for an anisotropy `g`.
fn phase(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (12.566371 * pow(max(1.0 + g2 - 2.0 * g * cos_theta, 1e-4), 1.5));
}

struct FogRay {
    start: vec3<f32>,
    direction: vec3<f32>,
    // metres along the ray per metre of view depth
    stretch: f32,
    // the view depth at `start`
    start_depth: f32,
};

/// The ray through a place on the screen (0..1 across).
fn fog_ray(uv: vec2<f32>) -> FogRay {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let a = frame.inverse_view_projection * vec4<f32>(ndc, 0.0, 1.0);
    let b = frame.inverse_view_projection * vec4<f32>(ndc, 1.0, 1.0);
    let start = a.xyz / a.w;
    let direction = normalize(b.xyz / b.w - start);
    let per_metre = max(-dot(frame.view_depth.xyz, direction), 1e-4);
    return FogRay(start, direction, 1.0 / per_metre, -dot(frame.view_depth, vec4<f32>(start, 1.0)));
}

// What the air in each cell scatters towards the eye: the sun through its
// cascades, the lamps of the cell's cluster through their maps, and the
// sky from all round; its density in alpha.
@compute @workgroup_size(4, 4, 4)
fn cs_fog_inject(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id >= FOG_SIZE) {
        return;
    }
    let cell = vec3<f32>(id) + 0.5;
    let uv = cell.xy / vec2<f32>(FOG_SIZE.xy);
    let ray = fog_ray(uv);
    let depth = fog_depth(cell.z / f32(FOG_SIZE.z));
    let p = ray.start + ray.direction * (depth - ray.start_depth) * ray.stretch;
    let air = fog_density(p);
    let dust = puff_dust(p);
    let smoke = smoke_at(p);
    let density = air + dust.a + smoke.extinction;
    let to_eye = -ray.direction;
    let g = frame.fog_shape.z;

    let to_sun = -normalize(frame.sun_direction.xyz);
    // The sun into the cell: by its cascades, or by rays from three points
    // spread through it and turned each frame, so a shaft through a lattice
    // keeps its bars and TAA smooths what the grid is too coarse for.
    var sun_seen = 0.0;
    if frame.ray.x > 0.5 {
        let turn = frame.ambient_occlusion.w;
        for (var k = 0u; k < 3u; k = k + 1u) {
            let o = fract(vec3<f32>(0.1731, 0.5329, 0.8971) * f32(k + 1u) + turn + pixel_noise(vec2<f32>(id.xy) + f32(id.z) * 7.0)) - 0.5;
            let r = fog_ray((cell.xy + o.xy) / vec2<f32>(FOG_SIZE.xy));
            let at = fog_depth((cell.z + o.z) / f32(FOG_SIZE.z));
            let q = r.start + r.direction * (at - r.start_depth) * r.stretch;
            sun_seen += ray_visible(q, to_sun, 1.0e4);
        }
        sun_seen = sun_seen / 3.0;
    } else {
        sun_seen = sunlight(p, vec3<f32>(0.0));
    }
    let sun_through = frame.sun_color.rgb * sun_seen;
    var light = sun_through * phase(dot(-to_sun, to_eye), g);
    // The sky's light, from every way at once.
    light += mix(frame.ground_color.rgb, frame.sky_color.rgb, 0.5) * frame.fog_shape.w;

    let cell_of = light_cells[light_cell(uv * frame.cluster_depth.zw, p)];
    for (var n = 0u; n < cell_of.y; n = n + 1u) {
        let lamp = lights[light_indices[cell_of.x + n]];
        let to_lamp = lamp.position_range.xyz - p;
        let distance_to = length(to_lamp);
        let toward = to_lamp / max(distance_to, 1e-4);
        let reach = clamp(1.0 - distance_to / lamp.position_range.w, 0.0, 1.0);
        let spot = lamp.spot;
        let edge = spot.w + (1.0 - spot.w) * 0.1;
        let cone = select(smoothstep(spot.w, edge, dot(-toward, spot.xyz)), 1.0, spot.w < -1.5);
        if reach * cone <= 0.0 {
            continue;
        }
        var shadow = 1.0;
        if frame.ray.y > 0.5 {
            shadow = ray_visible(p, toward, max(distance_to - 0.1, 0.0));
        } else {
            shadow = lamp_shadow(lamp, p, vec3<f32>(0.0), distance_to);
        }
        // Closer to a square law than the surfaces' soft pool: a lamp in
        // mist is a glow round the lamp, not an even wash to its range.
        let near_lamp = 1.0 / (1.0 + distance_to * distance_to);
        light += lamp.color_shadow.rgb * reach * reach * near_lamp * cone * shadow
            * phase(dot(-toward, to_eye), g) * frame.fog_lamps.x;
    }
    // Kicked-up dust is thick enough to light as a soft solid would — the
    // light bouncing about inside it, not the thin air's single turn
    // towards the eye — and from the ground and sky round it.
    let dust_light = sun_through * 0.45 + mix(frame.ground_color.rgb, frame.sky_color.rgb, 0.5) * 0.9;
    // Smoke is lit as the dust is, in its own colour; fire glows by its
    // heat, whatever lights it.
    let smoke_light = dust_light * smoke.scatter + smoke.glow;
    textureStore(fog_scatter_out, id, vec4<f32>(light * frame.fog_medium.rgb * air + dust_light * dust.rgb + smoke_light, density));
}

// Each column front to back: what each cell adds, dimmed by what lies
// before it, and what is let through so far (Frostbite's energy-
// conserving step).
@compute @workgroup_size(8, 8, 1)
fn cs_fog_integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= FOG_SIZE.x || id.y >= FOG_SIZE.y {
        return;
    }
    let ray = fog_ray((vec2<f32>(id.xy) + 0.5) / vec2<f32>(FOG_SIZE.xy));
    var added = vec3<f32>(0.0);
    var through = 1.0;
    var previous = frame.volume.z;
    for (var z = 0u; z < FOG_SIZE.z; z = z + 1u) {
        let far_edge = fog_depth(f32(z + 1u) / f32(FOG_SIZE.z));
        let cell = textureLoad(fog_scatter_in, vec3<i32>(vec3<u32>(id.xy, z)), 0);
        let extinction = max(cell.a, 1e-6);
        let passed = exp(-extinction * (far_edge - previous) * ray.stretch);
        added += through * (cell.rgb - cell.rgb * passed) / extinction;
        through *= passed;
        textureStore(fog_integrated_out, vec3<u32>(id.xy, z), vec4<f32>(added, through));
        previous = far_edge;
    }
}

/// A colour seen through the fog between it and the eye: dimmed by what
/// the air takes, and the air's own light added.
fn through_fog(color: vec3<f32>, pixel: vec2<f32>, depth: f32) -> vec3<f32> {
    if frame.volume.x < 0.5 {
        return color;
    }
    let uv = pixel / frame.cluster_depth.zw;
    let near = frame.volume.z;
    let t = log(max(depth, near) / near) / log(frame.volume.y / near);
    // Each cell holds the sum to its far edge.
    let w = clamp(t - 0.5 / f32(FOG_SIZE.z), 0.0, 1.0);
    let fog = textureSampleLevel(fog_volume, fog_sampler, vec3<f32>(uv, w), 0.0);
    return color * fog.a + fog.rgb;
}

fn hash21(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(123.34, 456.21));
    let r = q + dot(q, q + 45.32);
    return fract(r.x * r.y);
}

/// Smooth noise, 0 to 1.
fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

/// Where water and snow gather on the ground: broad patches, a finer
/// edge.
fn patches(p: vec2<f32>) -> f32 {
    return value_noise(p * 0.35) * 0.65 + value_noise(p * 1.3) * 0.35;
}

/// A puddle's surface in the rain: rings spreading where drops land, two
/// staggered grids of them, each ring born, growing and fading.
fn ripples(p: vec2<f32>, t: f32, rain: f32) -> vec3<f32> {
    var slope = vec2<f32>(0.0);
    for (var k = 0; k < 2; k = k + 1) {
        let q = p / 0.35 + vec2<f32>(f32(k) * 0.5);
        let cell = floor(q);
        let born = hash21(cell + f32(k) * 7.0);
        let centre = cell + vec2<f32>(hash21(cell + 3.1), hash21(cell + 5.7));
        let age = fract(t * 1.3 + born);
        let away = q - centre;
        let d = length(away);
        let x = (d - age * 0.7) * 12.0;
        let wave = select(0.0, sin(x * 3.14159) * (1.0 - abs(x)), abs(x) < 1.0) * (1.0 - age);
        slope += away / max(d, 1e-3) * wave * rain;
    }
    return normalize(vec3<f32>(-slope.x * 0.25, 1.0, -slope.y * 0.25));
}

struct Weathered {
    albedo: vec3<f32>,
    smoothness: f32,
    normal: vec3<f32>,
    // what is left of the surface's metal under water or snow
    metal: f32,
};

/// A surface as the weather leaves it: darker and shinier wet, still water
/// in the level patches, snow on what faces up.
fn weathered(albedo: vec3<f32>, smoothness: f32, normal: vec3<f32>, geometric: vec3<f32>, position: vec3<f32>, pixel: vec2<f32>, is_sand: bool, clay: bool) -> Weathered {
    var out = Weathered(albedo, smoothness, normal, 1.0);
    let w = frame.weather[0];
    // Sand the wind has laid: on what faces up, thick in corners and
    // crevices (where the light from all round cannot get in either), and
    // against what faces into the wind — ragged at its edges. Not on sand
    // itself: it is what the drift is made of.
    let drifted = select(frame.weather[2].x, 0.0, is_sand);
    if drifted > 0.0 {
        var tucked = 0.0;
        if frame.ambient_occlusion.x > 0.5 {
            tucked = 1.0 - textureLoad(occlusion, vec2<i32>(pixel), 0).a;
        }
        var wd = vec2<f32>(frame.foliage.wind.x, frame.foliage.wind.y);
        if dot(wd, wd) < 1e-6 {
            wd = vec2<f32>(1.0, 0.0);
        }
        wd = normalize(wd);
        let into_wind = max(dot(geometric, -vec3<f32>(wd.x, 0.0, wd.y)), 0.0) * (1.0 - abs(geometric.y));
        let lying = smoothstep(0.25, 0.85, geometric.y) * 0.65 + tucked * 3.5 + into_wind * 0.3;
        // Tongues of sand drawn out along the wind.
        let along = vec2<f32>(dot(position.xz, wd) * 0.45, dot(position.xz, vec2<f32>(-wd.y, wd.x)) * 1.8);
        let ragged = patches(along + 71.0) * 0.65 + patches(along * 3.1 + 3.0) * 0.35;
        let amount = clamp(lying, 0.0, 1.0) * drifted;
        let sand = smoothstep(1.0 - amount, 1.0 - amount + 0.12, ragged * 0.85 + 0.15) * step(0.001, amount);
        out.albedo = mix(out.albedo, vec3<f32>(0.58, 0.34, 0.15), sand);
        out.smoothness = mix(out.smoothness, 0.08, sand);
        out.normal = normalize(mix(out.normal, geometric, sand * 0.7));
        out.metal = out.metal * (1.0 - sand);
    }
    // Drying after the rain: not evenly but in patches — the open and the
    // high first, what lies low last. How wet it still is here.
    let drying = frame.weather[2].y;
    var damp = 1.0;
    if drying > 0.0 {
        // Its place in the queue, 0 to 1: patchy, and a little earlier the
        // higher it stands. It is dry once the drying has got past it.
        let order = clamp(patches(position.xz * 1.4 + 5.0) * 0.65 + patches(position.xz * 4.0 + 9.0) * 0.35
            - clamp(position.y * 0.05, -0.1, 0.1), 0.0, 1.0);
        damp = 1.0 - smoothstep(order - 0.08, order + 0.08, drying * 1.16 - 0.08);
    }
    let wet_here = w.x * damp;
    let puddles = w.y * (1.0 - drying);
    // Clay: mud while it is wet, and as it dries lighter, and cracking.
    if clay {
        let dry = 1.0 - wet_here;
        let crack = crackle(position.xz * 3.2);
        let width = 0.02 + 0.13 * dry * dry;
        let gap = (1.0 - smoothstep(width * 0.4, width, crack.x)) * smoothstep(0.35, 0.7, dry);
        out.albedo = out.albedo * mix(0.55, 0.95, dry) * mix(1.0, 0.12, gap);
        out.smoothness = mix(out.smoothness, 0.05, dry);
        // Each plate curls a little as it shrinks: its edges lift.
        let curl = (1.0 - smoothstep(0.0, 0.5, crack.x)) * dry * 0.6;
        out.normal = normalize(out.normal + vec3<f32>(crack.y, 0.0, crack.z) * curl);
    }
    // Mud splashed up the foot of things: brown and dull in spatters,
    // thickest low, gone by the mud's height; wetter where it is wet.
    let mud = frame.weather[2].z;
    let mud_height = frame.weather[2].w;
    if mud > 0.0 && mud_height > 0.0 && !is_sand {
        let low = 1.0 - smoothstep(0.0, mud_height, position.y);
        let spatter = patches(position.xz * 6.0 + vec2<f32>(position.y * 9.0, 3.0)) * 0.6 + patches(position.xz * 17.0 + position.y * 23.0) * 0.4;
        let amount = mud * low * (1.0 - 0.6 * max(geometric.y, 0.0));
        let splash = smoothstep(1.0 - amount, 1.0 - amount + 0.1, spatter) * step(0.001, amount);
        out.albedo = mix(out.albedo, vec3<f32>(0.18, 0.12, 0.07), splash);
        out.smoothness = mix(out.smoothness, 0.1 + 0.5 * wet_here, splash);
        out.metal = out.metal * (1.0 - splash);
    }
    if wet_here + puddles + w.z <= 0.0 {
        return out;
    }
    let up = clamp(geometric.y, 0.0, 1.0);
    // Wet: what soaks darkens, and everything shines — up-facing most. Wet
    // sand is dark but dull: its grains break the shine.
    let wet = wet_here * (0.5 + 0.5 * up);
    out.albedo = out.albedo * mix(1.0, 0.55, wet);
    out.smoothness = mix(out.smoothness, select(0.85, 0.45, is_sand), wet * 0.8);
    // Puddles: on what is level, in patches that grow with the amount.
    let level = smoothstep(0.92, 0.98, geometric.y);
    let puddle = level * smoothstep(1.0 - puddles, 1.0 - puddles + 0.08, patches(position.xz)) * step(0.001, puddles);
    if puddle > 0.0 {
        out.albedo = out.albedo * mix(1.0, 0.35, puddle);
        out.smoothness = mix(out.smoothness, 1.0, puddle);
        let water = ripples(position.xz, frame.foliage.wind.w, w.w);
        out.normal = normalize(mix(out.normal, water, puddle));
        out.metal = 1.0 - puddle;
    }
    // Snow: on what faces up, patchy until it is whole.
    let facing = smoothstep(0.3, 0.8, geometric.y);
    let cover = patches(position.xz * 1.7 + 13.0) * 0.7 + 0.3;
    let snow = facing * smoothstep(1.0 - w.z, 1.0 - w.z + 0.15, cover) * step(0.001, w.z);
    out.albedo = mix(out.albedo, vec3<f32>(0.9, 0.92, 0.95), snow);
    out.smoothness = mix(out.smoothness, 0.25, snow);
    out.normal = normalize(mix(out.normal, geometric, snow * 0.8));
    out.metal = out.metal * (1.0 - snow);
    return out;
}

/// Cracked ground at `p` (cells about a unit across): how far from the
/// nearest crack, 0 on it; and which way is away from it, the plate's
/// middle, level.
fn crackle(p: vec2<f32>) -> vec3<f32> {
    let cell = floor(p);
    var first = 8.0;
    var second = 8.0;
    var middle = vec2<f32>(0.0);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let c = cell + vec2<f32>(f32(x), f32(y));
            let at = c + vec2<f32>(cloud_hash(vec3<f32>(c, 3.0)), cloud_hash(vec3<f32>(c, 7.0))) * 0.8 + 0.1;
            let d = length(p - at);
            if d < first {
                second = first;
                first = d;
                middle = at;
            } else if d < second {
                second = d;
            }
        }
    }
    let toward = normalize(middle - p + vec2<f32>(1e-5));
    return vec3<f32>(second - first, -toward.x, -toward.y);
}

/// What a terrain is drawn with: its instance's numbers, from the vertex
/// input or, for the mesh shader, from the frame.
struct TerrainLook {
    color_and_shading: vec4<f32>,
    surface: vec4<f32>,
    emission: vec4<f32>,
    uv_transform: vec4<f32>,
    detail: vec4<f32>,
    params_0: vec4<f32>,
    params_1: vec4<f32>,
};

// terrain-stage:begin (copied for the mesh shader, frame -> tframe)
/// The terrain's own height at `local` (metres in its space, from its
/// middle): its grid of heights, between four of them; 0 off it.
fn relief_height(local: vec2<f32>) -> f32 {
    let size = frame.terrain.x;
    let cells = frame.terrain.y;
    let g = (local / size + 0.5) * cells;
    if any(g < vec2<f32>(0.0)) || any(g > vec2<f32>(cells)) {
        return 0.0;
    }
    let i = min(floor(g), vec2<f32>(cells - 1.0));
    let f = g - i;
    let at = vec2<i32>(i);
    let h00 = textureLoad(terrain_heights, at, 0).r;
    let h10 = textureLoad(terrain_heights, at + vec2<i32>(1, 0), 0).r;
    let h01 = textureLoad(terrain_heights, at + vec2<i32>(0, 1), 0).r;
    let h11 = textureLoad(terrain_heights, at + vec2<i32>(1, 1), 0).r;
    return mix(mix(h00, h10, f.x), mix(h01, h11, f.x), f.y);
}

/// The wind's level way, for sand.
fn sand_wind() -> vec2<f32> {
    var w = vec2<f32>(frame.foliage.wind.x, frame.foliage.wind.y);
    if dot(w, w) < 1e-6 {
        w = vec2<f32>(1.0, 0.0);
    }
    return normalize(w);
}

/// The height of sand's ripples at `xz` in metres: the fine ones as much
/// as `fine`, the big ones they ride on as much as `coarse` — the shapes
/// the sand shader's normals draw, here as ground to stand on.
fn ripples_at(xz: vec2<f32>, fine: f32, coarse: f32) -> f32 {
    let w = sand_wind();
    let side = vec2<f32>(-w.y, w.x);
    var h = 0.0;
    if fine > 0.0 {
        h += ripple_height(dot(xz, w) / 0.14, dot(xz, side) / 0.14) * 0.012 * fine;
    }
    if coarse > 0.0 {
        h += ripple_height(dot(xz, w) / 0.6 + 7.3, dot(xz, side) / 0.6) * 0.025 * coarse;
    }
    return h;
}

/// The terrain's ground at world `xz`: its height, and the ripples on it
/// where the grid is fine enough to hold them (`fine`, `coarse`) and the
/// ground level enough (`flat`).
fn terrain_ground(xz: vec2<f32>, fine: f32, coarse: f32, sand: bool) -> f32 {
    let local = (frame.terrain_to_local * vec4<f32>(xz.x, 0.0, xz.y, 1.0)).xz;
    let h = relief_height(local);
    var y = (frame.terrain_to_world * vec4<f32>(local.x, h, local.y, 1.0)).y;
    if sand {
        // Ripples keep off slopes too steep to hold them, as the sand
        // shader's do: the slope from the heights round the point.
        let e = frame.terrain.x / frame.terrain.y;
        let sx = relief_height(local + vec2<f32>(e, 0.0)) - relief_height(local - vec2<f32>(e, 0.0));
        let sz = relief_height(local + vec2<f32>(0.0, e)) - relief_height(local - vec2<f32>(0.0, e));
        let up = 1.0 / sqrt(1.0 + (sx * sx + sz * sz) / (4.0 * e * e));
        let flat_enough = smoothstep(0.7, 0.93, up);
        y += ripples_at(xz, fine * flat_enough, coarse * flat_enough);
    }
    return y;
}

/// A vertex of terrain's fine grid: cell `g` of ring `level`, placed round
/// the camera and raised (see [`vs_terrain`]).
fn terrain_vertex(g: vec2<f32>, level: f32, look: TerrainLook) -> VertexOutput {
    let spacing = frame.terrain.w * exp2(level);
    let eye = frame.camera_position.xz;
    let snap = spacing * 2.0;
    let centre = floor(eye / snap) * snap;
    let first = centre + g * spacing;
    // Toward the ring's edge, odd vertices fold onto their even
    // neighbours: the next ring out has only those. Folded fully by 62
    // cells out, so the overlap with the next ring — to 66 — is the next
    // ring's own grid, triangle for triangle.
    let d = max(abs(first.x - eye.x), abs(first.y - eye.y)) / spacing;
    let morph = clamp((d - 44.0) / 18.0, 0.0, 1.0);
    let folded = g - fract(g * 0.5) * 2.0 * morph;
    let xz = centre + folded * spacing;
    let span = spacing * (1.0 + morph);
    // Ripples fourteen centimetres apart need a vertex every three or
    // four; the big ones, sixty apart, every twelve or so.
    let fine = 1.0 - smoothstep(0.02, 0.045, span);
    let coarse = 1.0 - smoothstep(0.1, 0.2, span);
    let sand = look.color_and_shading.w > 3.5;
    let y = terrain_ground(xz, fine, coarse, sand);
    let e = max(span, 0.01);
    let hx = terrain_ground(xz + vec2<f32>(e, 0.0), fine, coarse, sand) - terrain_ground(xz - vec2<f32>(e, 0.0), fine, coarse, sand);
    let hz = terrain_ground(xz + vec2<f32>(0.0, e), fine, coarse, sand) - terrain_ground(xz - vec2<f32>(0.0, e), fine, coarse, sand);
    let normal = normalize(vec3<f32>(-hx / (2.0 * e), 1.0, -hz / (2.0 * e)));
    // Off the terrain the grid is not drawn: whatever ground is round it
    // shows (a threshold past any alpha cuts it away).
    let local = (frame.terrain_to_local * vec4<f32>(xz.x, 0.0, xz.y, 1.0)).xz;
    let outside = any(abs(local) > vec2<f32>(frame.terrain.x * 0.5));

    var out: VertexOutput;
    let world = vec3<f32>(xz.x, y, xz.y);
    out.clip_position = frame.view_projection * vec4<f32>(world, 1.0);
    out.world_position = world;
    out.normal = normal;
    out.base_color = look.color_and_shading.rgb;
    out.shading = look.color_and_shading.w;
    out.uv = xz * look.uv_transform.xy + look.uv_transform.zw;
    out.surface = vec4<f32>(look.surface.xyz, select(0.0, 2.0, outside));
    out.emission = look.emission;
    out.detail = look.detail;
    out.params_0 = look.params_0;
    // How much of the ripples the geometry holds: the sand shader draws
    // only the rest.
    out.params_1 = vec4<f32>(look.params_1.xy, fine, coarse);
    out.subsurface = vec4<f32>(0.0, 0.0, 0.0, 0.01);
    out.maps = vec4<u32>(0u);
    return out;
}

// terrain-stage:end

/// Terrain's fine grid round the camera — tessellation, done as the GPU
/// allows it: rings of 128 cells, each twice the spacing of the one inside
/// it, from three centimetres at one's feet; each ring snapped to its own
/// grid as the camera moves, its outer edge folding into the next ring's
/// spacing so no crack opens between them. Every vertex raised to the
/// terrain's height and, where the grid is fine enough to hold them, to
/// the sand's ripples — real relief, catching the light and standing out
/// against the sky. Its normal is the slope of what it was raised to.
@vertex
fn vs_terrain(in: VertexInput) -> VertexOutput {
    var out = terrain_vertex(
        in.position.xz,
        in.position.y,
        TerrainLook(in.color_and_shading, in.surface, in.emission, in.uv_transform, in.detail, in.params_0, in.params_1),
    );
    out.maps = in.maps;
    return out;
}

@vertex
fn vs(in: VertexInput) -> VertexOutput {
    return standard_vertex(in);
}

// Tools over the finished picture (tools.rs): handles and the outline
// mask. Nothing of the world's light, fog or maps — a handle is the same
// colour whatever the scene is doing.
struct ToolOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // rgb, and alpha
    @location(2) color: vec4<f32>,
    // 0 shaded (a cone, a cube), 1 flat (a line, a square)
    @location(3) shading: f32,
    @location(4) clip: vec4<f32>,
};

@vertex
fn vs_tool(in: VertexInput) -> ToolOut {
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    let world = model * vec4<f32>(in.position, 1.0);
    var out: ToolOut;
    out.clip_position = frame.view_projection * world;
    out.clip = out.clip_position;
    out.world_position = world.xyz;
    out.normal = (model * vec4<f32>(in.normal, 0.0)).xyz;
    out.color = vec4<f32>(in.color_and_shading.rgb, in.surface.z);
    out.shading = in.color_and_shading.w;
    return out;
}

@fragment
fn fs_tool(in: ToolOut) -> @location(0) vec4<f32> {
    var rgb = in.color.rgb;
    if in.shading < 0.5 {
        // Lit, for a tool, is by a lamp over the eye's shoulder, as Unity
        // shades its handles: a cone reads as a cone from any side, and
        // no sun or shadow changes it.
        let n = normalize(in.normal);
        let to_eye = normalize(frame.camera_position.xyz - in.world_position);
        let lamp = normalize(to_eye + vec3<f32>(0.0, 0.7, 0.0));
        let facing = clamp(dot(n, lamp), 0.0, 1.0);
        rgb = rgb * (0.45 + 0.6 * facing) + vec3<f32>(0.18) * pow(clamp(dot(n, to_eye), 0.0, 1.0), 16.0);
    }
    let a = clamp(in.color.a, 0.0, 1.0);
    return vec4<f32>(rgb * a, a);
}

// What an outline is drawn round: red the colour's number, green 1 where
// nothing the prepass saw is in front of it.
@fragment
fn fs_outline_mask(in: ToolOut) -> @location(0) vec4<f32> {
    let ndc = in.clip.xy / in.clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let size = vec2<i32>(frame.cluster_depth.zw);
    let pixel = clamp(vec2<i32>(uv * frame.cluster_depth.zw), vec2<i32>(0), size - vec2<i32>(1));
    let d = textureLoad(scene_depth, pixel, 0);
    // Compared as depth along the view, in metres, for either projection.
    let seen = frame.inverse_view_projection * vec4<f32>(ndc, d, 1.0);
    let seen_depth = -dot(frame.view_depth, vec4<f32>(seen.xyz / seen.w, 1.0));
    let mine = -dot(frame.view_depth, vec4<f32>(in.world_position, 1.0));
    let visible = select(0.0, 1.0, mine <= seen_depth * 1.01 + 0.03);
    return vec4<f32>(in.color.r, visible, 0.0, 1.0);
}

// Clusters (cluster.rs): a dense mesh drawn a cluster at a time, the ones
// the compute pass kept, each an instance of 372 vertices whose vertex and
// instance are read out of the mesh's own buffers.
struct ClusterDrawn {
    instance: u32,
    first: u32,
    count: u32,
    pad: u32,
};

@group(3) @binding(3) var<storage, read> cluster_vertices: array<f32>;
@group(3) @binding(4) var<storage, read> cluster_indices: array<u32>;
@group(3) @binding(5) var<storage, read> cluster_instances: array<vec4<f32>>;
@group(3) @binding(6) var<storage, read> cluster_drawn: array<ClusterDrawn>;

@vertex
fn vs_cluster(@builtin(vertex_index) corner: u32, @builtin(instance_index) kept: u32) -> VertexOutput {
    let d = cluster_drawn[kept];
    if corner >= d.count * 3u {
        // Past the cluster's last triangle: all three corners the same
        // point, behind the eye — nothing drawn.
        var none: VertexOutput;
        none.clip_position = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        return none;
    }
    let v = cluster_indices[d.first + corner] * 8u;
    let s = d.instance * 13u;
    var in: VertexInput;
    in.position = vec3<f32>(cluster_vertices[v], cluster_vertices[v + 1u], cluster_vertices[v + 2u]);
    in.normal = vec3<f32>(cluster_vertices[v + 3u], cluster_vertices[v + 4u], cluster_vertices[v + 5u]);
    in.uv = vec2<f32>(cluster_vertices[v + 6u], cluster_vertices[v + 7u]);
    in.model_0 = cluster_instances[s];
    in.model_1 = cluster_instances[s + 1u];
    in.model_2 = cluster_instances[s + 2u];
    in.model_3 = cluster_instances[s + 3u];
    in.color_and_shading = cluster_instances[s + 4u];
    in.surface = cluster_instances[s + 5u];
    in.emission = cluster_instances[s + 6u];
    in.uv_transform = cluster_instances[s + 7u];
    in.detail = cluster_instances[s + 8u];
    in.params_0 = cluster_instances[s + 9u];
    in.params_1 = cluster_instances[s + 10u];
    in.subsurface = cluster_instances[s + 11u];
    in.maps = bitcast<vec4<u32>>(cluster_instances[s + 12u]);
    return standard_vertex(in);
}

fn standard_vertex(in: VertexInput) -> VertexOutput {
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    let world = vec4<f32>(
        trampled(
            swayed((model * vec4<f32>(in.position, 1.0)).xyz, in.model_3.xyz, in.detail.z, frame.foliage),
            in.model_3.xyz,
            in.detail.z,
            frame.foliage,
        ),
        1.0,
    );

    var out: VertexOutput;
    out.clip_position = frame.view_projection * world;
    out.world_position = world.xyz;
    // Assumes uniform scale. A non-uniform one would need the inverse
    // transpose; scenes that want it can scale at import instead, which is
    // where scale is settled anyway.
    out.normal = (model * vec4<f32>(in.normal, 0.0)).xyz;
    out.base_color = in.color_and_shading.rgb;
    out.shading = in.color_and_shading.w;
    out.uv = in.uv * in.uv_transform.xy + in.uv_transform.zw;
    out.surface = in.surface;
    out.emission = in.emission;
    out.detail = in.detail;
    out.params_0 = in.params_0;
    out.params_1 = in.params_1;
    out.subsurface = in.subsurface;
    out.maps = in.maps;
    return out;
}

// URP's Lit, the metallic workflow, with URP's own terms: the diffuse and
// specular colours from albedo and metallic, a GGX-shaped highlight
// normalised the way URP's DirectBRDFSpecular is, and the environment's
// reflection with URP's fresnel and roughness falloff. Light colours carry
// no 1/pi, as in Unity: a white light of intensity one on white paper is
// white.
struct Brdf {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
    perceptual_roughness: f32,
    roughness2: f32,
    normalization: f32,
    grazing: f32,
};

fn brdf(albedo: vec3<f32>, metallic: f32, smoothness: f32) -> Brdf {
    var out: Brdf;
    let one_minus_reflectivity = 0.96 - metallic * 0.96;
    out.diffuse = albedo * one_minus_reflectivity;
    out.specular = mix(vec3<f32>(0.04), albedo, metallic);
    out.perceptual_roughness = 1.0 - smoothness;
    let roughness = max(out.perceptual_roughness * out.perceptual_roughness, 0.0078125);
    out.roughness2 = roughness * roughness;
    out.normalization = roughness * 4.0 + 2.0;
    out.grazing = clamp(smoothness + 1.0 - one_minus_reflectivity, 0.0, 1.0);
    return out;
}

fn direct(b: Brdf, normal: vec3<f32>, to_light: vec3<f32>, to_eye: vec3<f32>, highlights: bool) -> vec3<f32> {
    var color = b.diffuse;
    if highlights {
        let half_way = normalize(to_light + to_eye);
        let n_h = max(dot(normal, half_way), 0.0);
        let l_h = max(dot(to_light, half_way), 0.0);
        let d = n_h * n_h * (b.roughness2 - 1.0) + 1.00001;
        let term = b.roughness2 / ((d * d) * max(0.1, l_h * l_h) * b.normalization);
        color = color + b.specular * term;
    }
    return color;
}

/// What the surroundings look like in a direction, blurred by roughness:
/// the sky's gradient for a smooth surface, the hemisphere's average for a
/// rough one. No sun in it: the sun is a direct light, counted once.
fn environment(direction: vec3<f32>, perceptual_roughness: f32) -> vec3<f32> {
    let hemisphere = mix(frame.ground_color.rgb, frame.sky_color.rgb, direction.y * 0.5 + 0.5);
    if frame.sky_zenith.w < 0.5 {
        return hemisphere;
    }
    if frame.sky_zenith.w > 1.5 {
        return mix(physical_sky(direction) * frame.sky_ground.w, hemisphere, perceptual_roughness);
    }
    var sky: vec3<f32>;
    if direction.y >= 0.0 {
        sky = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, pow(direction.y, 0.45));
    } else {
        sky = mix(frame.sky_horizon.rgb, frame.sky_ground.rgb, pow(-direction.y, 0.3));
    }
    return mix(sky * frame.sky_ground.w, hemisphere, perceptual_roughness);
}

/// A probe's picture the way `direction` looks, `lod` mips blurred.
fn probe_picture(probe: u32, direction: vec3<f32>, lod: f32) -> vec3<f32> {
    let a = abs(direction);
    var face = 0u;
    if a.x >= a.y && a.x >= a.z {
        face = select(1u, 0u, direction.x > 0.0);
    } else if a.y >= a.z {
        face = select(3u, 2u, direction.y > 0.0);
    } else {
        face = select(5u, 4u, direction.z > 0.0);
    }
    let clip = frame.probe_faces[face] * vec4<f32>(direction, 1.0);
    let ndc = clip.xy / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    return textureSampleLevel(probe_maps, probe_sampler, uv, i32(probe * 6u + face), lod).rgb;
}

/// What a surface at `position` reflects along `direction`: the probes
/// whose boxes hold it — bent to the box's walls where asked, fading out
/// inside each box's edge, the first ones first — and the sky for what
/// they leave.
fn reflected(position: vec3<f32>, direction: vec3<f32>, perceptual_roughness: f32) -> vec3<f32> {
    let around = probes_and_sky(position, direction, perceptual_roughness);
    // By a ray, where asked: what is off the screen or behind the camera
    // too. A rough surface's ray is turned a little every frame and pixel
    // within its lobe, and TAA gathers them into its blur.
    if frame.probe_params.z > 0.5 && perceptual_roughness <= frame.probe_params.w {
        var d = direction;
        let spread = perceptual_roughness * perceptual_roughness;
        if spread > 0.0005 {
            let turn = frame.ambient_occlusion.w;
            let n1 = fract(pixel_noise(position.xz * 97.0 + position.y * 13.0) + turn);
            let n2 = fract(pixel_noise(position.zy * 71.0 + position.x * 7.0) + turn * 1.7);
            let side = basis_of(direction);
            let a = n2 * 6.2831853;
            d = normalize(direction + (side.t * cos(a) + side.b * sin(a)) * sqrt(n1) * spread);
        }
        let traced = ray_reflection(position, d);
        if traced.a > 0.5 {
            return traced.rgb;
        }
        return around;
    }
    let near = screen_reflection(position, direction, perceptual_roughness);
    return mix(around, near.rgb, near.a);
}

/// The light from all round a surface facing `normal`: inside a reflection
/// probe's box, its most blurred picture that way — the room as it is lit,
/// its coloured walls tinting what faces them, even off the screen — and
/// outside, the sky and the ground's `hemisphere`.
fn around(position: vec3<f32>, normal: vec3<f32>, hemisphere: vec3<f32>) -> vec3<f32> {
    let count = u32(frame.probe_params.x);
    if count == 0u {
        return hemisphere;
    }
    let blurred = frame.probe_params.y;
    var sum = vec3<f32>(0.0);
    var covered = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let centre = frame.probes[i * 2u];
        let extents = frame.probes[i * 2u + 1u];
        let inside = extents.xyz - abs(position - centre.xyz);
        let edge = min(inside.x, min(inside.y, inside.z));
        if edge <= 0.0 {
            continue;
        }
        let weight = clamp(edge / max(centre.w, 1e-3), 0.0, 1.0) * (1.0 - covered);
        // The last mip is a few texels a face: already the average of what
        // a face sees; three taps round the normal smooth its seams.
        let t = basis_of(normal);
        let seen = probe_picture(i, normal, blurred) * 0.5
            + probe_picture(i, normalize(normal + t.t * 0.6), blurred) * 0.25
            + probe_picture(i, normalize(normal - t.t * 0.6), blurred) * 0.25;
        sum = sum + seen * weight;
        covered = covered + weight;
        if covered > 0.999 {
            break;
        }
    }
    return sum + hemisphere * (1.0 - covered);
}

// DDGI (ddgi.rs): each probe's two octahedral pictures of 8×8 texels,
// its light and then its distances (mean, mean of squares, 1 when the
// probe is out in the open).
@group(0) @binding(25) var<storage, read> ddgi_probes: array<vec4<f32>>;

const DDGI_TEXELS: u32 = 8u;
const DDGI_PER_PROBE: u32 = 128u;

/// A direction to a point of the unit square, by the octahedron.
fn oct_encode(d: vec3<f32>) -> vec2<f32> {
    var p = d.xz / (abs(d.x) + abs(d.y) + abs(d.z));
    if d.y < 0.0 {
        p = (1.0 - abs(p.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
    }
    return p * 0.5 + 0.5;
}

fn oct_decode(uv: vec2<f32>) -> vec3<f32> {
    let e = uv * 2.0 - 1.0;
    var d = vec3<f32>(e.x, 1.0 - abs(e.x) - abs(e.y), e.y);
    if d.y < 0.0 {
        let xz = (1.0 - abs(d.zx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), d.xz >= vec2<f32>(0.0));
        d = vec3<f32>(xz.x, d.y, xz.y);
    }
    return normalize(d);
}

/// A probe's picture at `uv`, filtered between its four nearest texels;
/// `offset` 0 for its light, 64 for its distances.
fn ddgi_texel(probe: u32, uv: vec2<f32>, offset: u32) -> vec4<f32> {
    let t = clamp(uv * f32(DDGI_TEXELS) - 0.5, vec2<f32>(0.0), vec2<f32>(f32(DDGI_TEXELS) - 1.0));
    let a = vec2<u32>(floor(t));
    let b = min(a + 1u, vec2<u32>(DDGI_TEXELS - 1u));
    let f = t - floor(t);
    let base = probe * DDGI_PER_PROBE + offset;
    let t00 = ddgi_probes[base + a.y * DDGI_TEXELS + a.x];
    let t10 = ddgi_probes[base + a.y * DDGI_TEXELS + b.x];
    let t01 = ddgi_probes[base + b.y * DDGI_TEXELS + a.x];
    let t11 = ddgi_probes[base + b.y * DDGI_TEXELS + b.x];
    return mix(mix(t00, t10, f.x), mix(t01, t11, f.x), f.y);
}

/// The diffuse light the irradiance volume gives a surface at `p`
/// turned `n`, seen from `to_eye`: rgb, and in alpha how much it covers
/// there (1 inside the grid and half a cell past it, 0 a cell past).
fn ddgi_irradiance(p: vec3<f32>, n: vec3<f32>, to_eye: vec3<f32>) -> vec4<f32> {
    if frame.ddgi[1].w < 0.5 {
        return vec4<f32>(0.0);
    }
    let origin = frame.ddgi[0].xyz;
    let spacing = frame.ddgi[0].w;
    let counts = vec3<i32>(frame.ddgi[1].xyz);
    let biased = p + n * frame.ddgi[2].x + to_eye * frame.ddgi[2].y;
    let g = (biased - origin) / spacing;
    let far = vec3<f32>(counts - 1);
    let inside = min(g, far - g);
    // Whole to half a cell past the outer probes — a room's walls, just
    // outside them, are theirs — and gone a cell past.
    let covered = clamp((min(inside.x, min(inside.y, inside.z)) + 1.0) * 2.0, 0.0, 1.0);
    if covered <= 0.0 {
        return vec4<f32>(0.0);
    }
    let base = clamp(vec3<i32>(floor(g)), vec3<i32>(0), counts - 2);
    let alpha = clamp(g - vec3<f32>(base), vec3<f32>(0.0), vec3<f32>(1.0));
    let normal_uv = oct_encode(n);
    var sum = vec3<f32>(0.0);
    var weights = 0.0;
    for (var i = 0u; i < 8u; i = i + 1u) {
        let offset = vec3<i32>(i32(i & 1u), i32((i >> 1u) & 1u), i32((i >> 2u) & 1u));
        let at = base + offset;
        let probe = u32(at.x + counts.x * (at.y + counts.y * at.z));
        let position = origin + vec3<f32>(at) * spacing;
        let near = mix(1.0 - alpha, alpha, vec3<f32>(offset));
        let trilinear = near.x * near.y * near.z;
        // On the side the surface faces: a probe behind it counts less.
        let toward = normalize(position - p);
        let facing = (dot(toward, n) + 1.0) * 0.5;
        var weight = facing * facing + 0.2;
        // Can the probe see it: its distances that way against how far.
        let away = biased - position;
        let r = length(away);
        let moments = ddgi_texel(probe, oct_encode(away / max(r, 1e-4)), 64u);
        if moments.z < 0.5 {
            continue;
        }
        if r > moments.x {
            let variance = abs(moments.x * moments.x - moments.y);
            let past = r - moments.x;
            let chebyshev = variance / (variance + past * past);
            weight *= max(chebyshev * chebyshev * chebyshev, 0.0);
        }
        // Crushed where it is faint, so a sliver of a hidden probe's light
        // does not come through.
        weight = max(weight, 1e-6);
        if weight < 0.2 {
            weight *= weight * weight / 0.04;
        }
        weight *= trilinear;
        sum += ddgi_texel(probe, normal_uv, 0u).rgb * weight;
        weights += weight;
    }
    // Inside the box, no probe that can see here is dark, not the sky:
    // the sky is what the probes found.
    if weights < 1e-5 {
        return vec4<f32>(0.0, 0.0, 0.0, covered);
    }
    return vec4<f32>(sum / weights, covered);
}

// The irradiance volume's update (ddgi.rs): this frame's rays, and the
// probes' pictures they are blended into.
struct DdgiStep {
    rotation: mat4x4<f32>,
    // rays a probe, how much of the old is kept, the farthest a distance
    // counts, probes
    rays: vec4<f32>,
};

@group(3) @binding(7) var<uniform> ddgi_step: DdgiStep;
@group(3) @binding(8) var<storage, read_write> ddgi_rays: array<vec4<f32>>;
@group(3) @binding(9) var<storage, read_write> ddgi_out: array<vec4<f32>>;

/// The `i`-th of a probe's rays this frame: spread evenly over the sphere
/// (spherical Fibonacci), turned the frame's way.
fn ddgi_direction(i: u32) -> vec3<f32> {
    let count = ddgi_step.rays.x;
    let k = f32(i) + 0.5;
    let y = 1.0 - 2.0 * k / count;
    let r = sqrt(max(1.0 - y * y, 0.0));
    let a = k * 2.3999632;
    return normalize((ddgi_step.rotation * vec4<f32>(cos(a) * r, y, sin(a) * r, 0.0)).xyz);
}

fn ddgi_position(probe: u32) -> vec3<f32> {
    let counts = vec3<u32>(frame.ddgi[1].xyz);
    let at = vec3<u32>(probe % counts.x, (probe / counts.x) % counts.y, probe / (counts.x * counts.y));
    return frame.ddgi[0].xyz + vec3<f32>(at) * frame.ddgi[0].w;
}

var<workgroup> ddgi_backs: atomic<u32>;

/// One probe a group, one texel of both its pictures a thread: every ray
/// of the frame weighed by how near it runs to the texel's direction.
@compute @workgroup_size(8, 8)
fn cs_ddgi_update(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_id) local: vec3<u32>) {
    let probe = group.x;
    let rays = u32(ddgi_step.rays.x);
    let keep = ddgi_step.rays.y;
    let farthest = ddgi_step.rays.z;
    let texel = local.y * DDGI_TEXELS + local.x;
    if texel == 0u {
        atomicStore(&ddgi_backs, 0u);
    }
    workgroupBarrier();
    let d = oct_decode((vec2<f32>(local.xy) + 0.5) / f32(DDGI_TEXELS));
    var light = vec3<f32>(0.0);
    var light_weight = 0.0;
    var mean = 0.0;
    var squares = 0.0;
    var distance_weight = 0.0;
    for (var i = 0u; i < rays; i = i + 1u) {
        let ray = ddgi_rays[probe * rays + i];
        let direction = ddgi_direction(i);
        let cosine = dot(d, direction);
        var t = ray.w;
        if t < 0.0 {
            // The back of a face: inside something. Nothing lit, and near.
            if texel == 0u {
                atomicAdd(&ddgi_backs, 1u);
            }
            t = -t * 0.2;
        } else {
            let w = max(cosine, 0.0);
            light += ray.rgb * w;
            light_weight += w;
        }
        let near = pow(max(cosine, 0.0), 50.0);
        let clamped = min(t, farthest);
        mean += clamped * near;
        squares += clamped * clamped * near;
        distance_weight += near;
    }
    workgroupBarrier();
    let open = select(0.0, 1.0, f32(atomicLoad(&ddgi_backs)) < f32(rays) * 0.25);
    let at = probe * DDGI_PER_PROBE + texel;
    let new_light = light / max(light_weight, 1e-4);
    let new_moments = vec2<f32>(mean, squares) / max(distance_weight, 1e-4);
    let old_light = ddgi_out[at];
    let old_moments = ddgi_out[at + 64u];
    ddgi_out[at] = vec4<f32>(mix(new_light, old_light.rgb, keep), 1.0);
    ddgi_out[at + 64u] = vec4<f32>(mix(new_moments, old_moments.xy, keep), open, 0.0);
}

/// What the probes and the sky give a reflection, without the screen.
fn probes_and_sky(position: vec3<f32>, direction: vec3<f32>, perceptual_roughness: f32) -> vec3<f32> {
    let sky = environment(direction, perceptual_roughness);
    let count = u32(frame.probe_params.x);
    if count == 0u {
        return sky;
    }
    // Unity's mip for a roughness: rougher reads blurrier, not linearly.
    let lod = perceptual_roughness * (1.7 - 0.7 * perceptual_roughness) * frame.probe_params.y;
    var sum = vec3<f32>(0.0);
    var covered = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let centre = frame.probes[i * 2u];
        let extents = frame.probes[i * 2u + 1u];
        let inside = extents.xyz - abs(position - centre.xyz);
        let edge = min(inside.x, min(inside.y, inside.z));
        if edge <= 0.0 {
            continue;
        }
        let weight = clamp(edge / max(centre.w, 1e-3), 0.0, 1.0) * (1.0 - covered);
        var look = direction;
        if extents.w > 0.5 {
            // Where the ray leaves the box, seen from the probe's centre.
            let far_wall = (sign(direction) * extents.xyz + centre.xyz - position) / direction;
            let leaves = min(far_wall.x, min(far_wall.y, far_wall.z));
            look = position + direction * leaves - centre.xyz;
        }
        sum = sum + probe_picture(i, normalize(look), lod) * weight;
        covered = covered + weight;
        if covered > 0.999 {
            break;
        }
    }
    return sum + sky * (1.0 - covered) * sky_share;
}

/// How much of the open sky a reflection here may show: less inside an
/// irradiance volume, by how much of the sky's light its probes found
/// gets in — a closed room's polish does not mirror a sky it cannot see.
/// Set by the lit shader for its pixel; 1 everywhere else.
var<private> sky_share: f32 = 1.0;

/// A tangent-space normal from the map, turned into the world. The
/// tangent frame is worked out from how position and UV change across the
/// pixel (Schüler's cotangent frame), so meshes need no tangents stored.
fn mapped_normal(geometric: vec3<f32>, world_position: vec3<f32>, uv: vec2<f32>, texel: vec3<f32>, scale: f32) -> vec3<f32> {
    let dp1 = dpdx(world_position);
    let dp2 = dpdy(world_position);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, geometric);
    let dp1perp = cross(geometric, dp1);
    // Screen y runs down here, where the cotangent frame was worked out
    // with it up: that turns the frame round, the tangent to its right way
    // and the bitangent to the image's up — UVs run down the image (its top
    // row first) while a normal map's green points up it, so the turned
    // bitangent is the one wanted.
    let t = -(dp2perp * duv1.x + dp1perp * duv2.x);
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let size = max(dot(t, t), dot(b, b));
    if size < 1e-12 {
        return geometric;
    }
    let inverse = inverseSqrt(size);
    var n = texel * 2.0 - 1.0;
    n = vec3<f32>(n.xy * scale, n.z);
    return normalize(t * inverse * n.x + b * inverse * n.y + geometric * n.z);
}

// What a material's own shader is given: where the fragment is, its
// surface's normal, its texture coordinates and the time.
struct SurfaceIn {
    world_position: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    time: f32,
    // The material's own eight numbers (`params` in its .rmat), in the
    // order its shader's `// runity:params` line names them.
    params: array<vec4<f32>, 2>,
    // Where its textures are, for `texture_at`: not to be read otherwise.
    maps: vec4<u32>,
};

// What the standard shader worked out for the fragment, before the light:
// what a material's shader changes.
struct Surface {
    albedo: vec3<f32>,
    alpha: f32,
    metallic: f32,
    smoothness: f32,
    normal: vec3<f32>,
    emission: vec3<f32>,
};

// A material's shader replaces this function — everything between the two
// marks — with its own `surface` (and whatever it needs above it).
// runity:surface {
fn surface(in: SurfaceIn, out: Surface) -> Surface {
    return out;
}
// runity:surface }

@fragment
fn fs(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return shade(in, front, true);
}

/// The solid scene over the prepass's own depth, drawn where it is equal:
/// what is cut out was cut there already, and a shader with no discard
/// keeps the GPU's hidden surface removal.
@fragment
fn fs_prepassed(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return shade(in, front, false);
}

fn shade(in: VertexOutput, front: bool, clip: bool) -> vec4<f32> {
    // The base map on the screen instead, for a camera's picture seen
    // through the surface: a mirror's (flipped) or a portal's.
    var base_uv = in.uv;
    let screen_flags = u32(in.emission.w + 0.5);
    if (screen_flags & 48u) != 0u {
        base_uv = in.clip_position.xy / frame.cluster_depth.zw;
        if (screen_flags & 32u) != 0u {
            base_uv.x = 1.0 - base_uv.x;
        }
    }
    let sampled = surface_at(in.maps, base_uv);
    let normal_texel = normal_at(in.maps, in.uv).xyz;
    let mask = mask_at(in.maps, in.uv);
    let emitted = emission_at(in.maps, in.uv).rgb;
    // A face seen from behind — a two-sided leaf — is lit from its own side.
    let geometric = normalize(in.normal) * select(-1.0, 1.0, front);
    var normal = mapped_normal(geometric, in.world_position, in.uv, normal_texel, in.detail.x);
    // How the position changes across the pixel: what a decal's picture is
    // filtered by, worked out here where every pixel still runs together.
    let across = dpdx(in.world_position);
    let down = dpdy(in.world_position);
    var alpha = in.surface.z * sampled.a;
    // Alpha clipping: what is less opaque than the threshold is not drawn
    // at all.
    if clip && in.surface.w > 0.0 && alpha < in.surface.w {
        discard;
    }
    let flags = u32(in.emission.w + 0.5);
    let unlit = f32(in.shading > 0.5 && in.shading < 1.5);
    let grid = f32(abs(in.shading - 2.0) < 0.5);
    let is_sand = in.shading > 3.5;

    var albedo = in.base_color * sampled.rgb * mix(1.0, metre_grid(in.world_position, normal), grid);
    var smoothness = in.surface.y * mask.a;
    let cell = light_cells[light_cell(in.clip_position.xy, in.world_position)];

    // Decals, before the light: what they paint is lit as the surface is.
    for (var n = 0u; n < cell.w; n = n + 1u) {
        let d = decals[light_indices[cell.z + n]];
        let local = (d.world_to_box * vec4<f32>(in.world_position, 1.0)).xyz;
        if any(abs(local) > vec3<f32>(0.5)) {
            continue;
        }
        let up = d.axis_up.xyz;
        // Only on what faces the way it is pressed, and fading towards the
        // box's ends, so it neither smears down sides nor stops at a line.
        var weight = d.color.a * smoothstep(0.1, 0.4, dot(geometric, up))
            * (1.0 - smoothstep(0.35, 0.5, abs(local.y)));
        let uv = vec2<f32>(local.x + 0.5, local.z + 0.5);
        let duv_x = (d.world_to_box * vec4<f32>(across, 0.0)).xz;
        let duv_y = (d.world_to_box * vec4<f32>(down, 0.0)).xz;
        if d.maps.x < -1.5 {
            // A footprint (footprints.rs): pressed in, a rim round it.
            let f = local.xz * 2.0;
            let e = 0.04;
            let h = footprint_height(f);
            let slope = vec2<f32>(
                footprint_height(f + vec2<f32>(e, 0.0)) - footprint_height(f - vec2<f32>(e, 0.0)),
                footprint_height(f + vec2<f32>(0.0, e)) - footprint_height(f - vec2<f32>(0.0, e)),
            ) / (2.0 * e);
            let pit = clamp(-h, 0.0, 1.0);
            let rim = clamp(h, 0.0, 1.0);
            albedo = mix(albedo, d.color.rgb, weight * pit);
            albedo = albedo * (1.0 + 0.15 * rim * weight);
            // Metres of height per metre across: the shape's slope through
            // the box into the world, times how deep it is.
            let into = mat3x3<f32>(d.world_to_box[0].xyz, d.world_to_box[1].xyz, d.world_to_box[2].xyz);
            var rise = transpose(into) * vec3<f32>(slope.x * 2.0, 0.0, slope.y * 2.0) * d.maps.z;
            rise = rise - up * dot(rise, up);
            normal = normalize(mix(normal, normalize(normal - rise), weight));
            continue;
        }
        var paint = d.color.rgb;
        if d.maps.x >= 0.0 {
            let texel = textureSampleGrad(decal_colours, probe_sampler, uv, i32(d.maps.x), duv_x, duv_y);
            paint = paint * texel.rgb;
            weight = weight * texel.a;
        }
        albedo = mix(albedo, paint, weight);
        smoothness = mix(smoothness, d.maps.w, weight);
        if d.maps.y >= 0.0 {
            var t = textureSampleGrad(decal_normals, probe_sampler, uv, i32(d.maps.y), duv_x, duv_y).xyz * 2.0 - 1.0;
            t = vec3<f32>(t.xy * d.maps.z, t.z);
            // Red along the box's x, green up the picture — its -z.
            let x = d.axis_x.xyz;
            let picture_up = normalize(cross(up, x));
            let pressed = normalize(x * t.x + picture_up * t.y + up * max(t.z, 1e-3));
            normal = normalize(mix(normal, pressed, weight));
        }
    }

    // Sand: the wind's ripples in it, and in a gale sand running over it.
    var glint_facet = vec3<f32>(0.0);
    if is_sand {
        let sand = sand_surface(in.world_position, normal, geometric, across, down, in.params_1.zw);
        normal = sand.normal;
        albedo = albedo * sand.shade;
        glint_facet = sand.glint;
    }

    // The weather on it: wet, under water, under snow.
    let weather = weathered(albedo, smoothness, normal, geometric, in.world_position, in.clip_position.xy, is_sand, (flags & 64u) != 0u);
    albedo = weather.albedo;
    smoothness = weather.smoothness;
    normal = weather.normal;

    // The material's own shader has its say, before the light.
    let shaped = surface(
        SurfaceIn(in.world_position, geometric, in.uv, frame.clear_color.w, array<vec4<f32>, 2>(in.params_0, in.params_1), in.maps),
        Surface(albedo, alpha, in.surface.x * mask.r * weather.metal, smoothness, normal, in.emission.rgb * emitted),
    );
    albedo = shaped.albedo;
    alpha = shaped.alpha;
    smoothness = shaped.smoothness;
    normal = normalize(shaped.normal);

    // An unlit surface is done here: what it emits, through the dust and
    // the fog, and none of the light's work below — the shadows, the
    // lamps, the sky — which it would only throw away. A draw is all one
    // or the other, so the branch costs nothing: a sky of smoke sprites,
    // each over most of the screen, is what this is for.
    if unlit > 0.5 {
        var lit_not = albedo + shaped.emission;
        if (frame.weather[1].z > 0.0 && dust_can_reach(in.world_position)) || frame.dust.x > 0.5 {
            let c = textureSampleLevel(cloud_layer, fog_sampler, in.clip_position.xy / frame.cluster_depth.zw, 0.0);
            lit_not = lit_not * c.a + c.rgb;
        }
        lit_not = through_fog(lit_not, in.clip_position.xy, -dot(frame.view_depth, vec4<f32>(in.world_position, 1.0)));
        if (flags & 8u) != 0u {
            lit_not = lit_not * alpha;
        }
        return vec4<f32>(lit_not, alpha);
    }

    let to_eye = normalize(frame.camera_position.xyz - in.world_position);
    let b = brdf(albedo, shaped.metallic, smoothness);
    let baked = mix(1.0, mask.g, in.detail.y);
    let highlights = (flags & 1u) != 0u;

    let to_sun = -normalize(frame.sun_direction.xyz);
    var shadow = 1.0;
    if (flags & 4u) != 0u {
        if frame.ray.x > 0.5 {
            shadow = traced_sun(in.world_position, geometric, to_sun, in.clip_position.xy);
        } else {
            shadow = sunlight(in.world_position, normal);
            if frame.ambient_occlusion.z > 0.0 && shadow > 0.0 {
                shadow *= contact_shadow(in.world_position, geometric, to_sun, in.clip_position.xy);
            }
        }
    }
    // The scene's distance field softens the sun's shadow where the map
    // is coarse and adds what the map missed.
    let field_on = frame.distance[0].w > 0.5 && unlit < 0.5;
    if field_on && shadow > 0.0 && (flags & 4u) != 0u {
        shadow = min(shadow, field_shadow(in.world_position, geometric, to_sun));
    }
    // Under a cloud: in its shadow.
    shadow *= cloud_shadow(in.world_position);
    // Under water: the sun comes down as caustics, dimmer the deeper.
    let submerged = under_water(in.world_position);
    if submerged > 0.0 {
        let pattern = caustics(in.world_position.xz, frame.foliage.wind.w);
        shadow *= (0.35 + 1.8 * pattern) * exp(-submerged * 0.35);
    }
    // Ambient occlusion darkens the light from all around, and a share of
    // the direct light too (URP's Direct Lighting Strength).
    // Its pass also gathers the light bounced off what is near (rgb).
    var ao = 1.0;
    var bounce = vec3<f32>(0.0);
    if frame.ray.z > 0.5 && unlit < 0.5 {
        ao = traced_occlusion(in.world_position, geometric, in.clip_position.xy);
    } else if frame.ambient_occlusion.x > 0.5 && unlit < 0.5 {
        let gathered = textureLoad(occlusion, vec2<i32>(in.clip_position.xy), 0);
        ao = gathered.a;
        bounce = gathered.rgb;
    }
    if field_on {
        ao *= field_occlusion(in.world_position, geometric);
    }
    let direct_ao = mix(1.0, ao, frame.ambient_occlusion.y);
    var color = direct(b, normal, to_sun, to_eye, highlights)
        * frame.sun_color.rgb * max(dot(normal, to_sun), 0.0) * shadow * direct_ao;
    // A grain of sand turned just so throws the sun straight at the eye.
    if dot(glint_facet, glint_facet) > 0.0 {
        // Its length is how much it shows, fading as the grains shrink
        // towards a pixel.
        let facet = normalize(glint_facet);
        let flash = pow(max(dot(reflect(-to_sun, facet), to_eye), 0.0), 250.0);
        color += frame.sun_color.rgb * shadow * flash * 12.0 * length(glint_facet)
            * max(dot(geometric, to_sun), 0.0);
    }
    // Light under the surface: past the edge of the lit side, the light
    // that went in on it comes out, tinted — a soft warm terminator on skin
    // rather than a hard grey one; and through what is thin toward the sun,
    // as thin as the shadow map says it is, fading with how far it went.
    let under = in.subsurface.rgb;
    if unlit < 0.5 && (under.r + under.g + under.b) > 0.0 {
        let n_l = dot(normal, to_sun);
        let wrap = 0.5;
        let wrapped = max((n_l + wrap) / (1.0 + wrap), 0.0);
        let spill = max(wrapped - max(n_l, 0.0), 0.0);
        color += under * frame.sun_color.rgb * spill * mix(1.0, shadow, 0.5) * direct_ao;
        let reach = max(in.subsurface.w, 0.0005);
        let thick = sun_thickness(in.world_position, geometric);
        if thick >= 0.0 {
            let through = exp(-thick / reach) * max(-n_l, 0.0);
            let into_sun = pow(max(dot(-to_eye, to_sun), 0.0), 4.0);
            color += under * frame.sun_color.rgb * through * (0.4 + 1.6 * into_sun);
        }
    }
    // Lit through from behind: a leaf, a blade of grass — brightest looking
    // straight at the sun through it.
    let translucency = in.detail.w;
    if translucency > 0.0 {
        let behind = max(dot(-geometric, to_sun), 0.0);
        let into_sun = pow(max(dot(-to_eye, to_sun), 0.0), 6.0);
        color += b.diffuse * frame.sun_color.rgb * shadow * translucency * (behind * 0.5 + into_sun * 1.5);
    }

    // Point and spot lights, those listed in this fragment's cell: facing
    // it, and fading to nothing at its range — squared, so the edge of the
    // pool is soft rather than a ring.
    // By ReSTIR, where asked: the one lamp this pixel's reservoir chose,
    // shadowed already, times its weight — all of them on average.
    let restir_on = frame.restir.x > 0.5 && (flags & 4u) != 0u;
    if restir_on {
        let pixel = vec2<u32>(in.clip_position.xy);
        let reservoir = restir_shade[pixel.y * u32(frame.restir.y) + pixel.x];
        if reservoir.w > 0.0 {
            let light = lights[bitcast<u32>(reservoir.x)];
            let to_light = light.position_range.xyz - in.world_position;
            let distance_to = length(to_light);
            let toward = to_light / max(distance_to, 1e-4);
            let reach = clamp(1.0 - distance_to / light.position_range.w, 0.0, 1.0);
            let facing = max(dot(normal, toward), 0.0);
            let along = dot(-toward, light.spot.xyz);
            let edge = light.spot.w + (1.0 - light.spot.w) * 0.1;
            let cone = select(smoothstep(light.spot.w, edge, along), 1.0, light.spot.w < -1.5);
            color = color + direct(b, normal, toward, to_eye, highlights)
                * light.color_shadow.rgb * facing * reach * reach * cone * direct_ao * reservoir.w;
        }
    }
    for (var n = 0u; n < select(cell.y, 0u, restir_on); n = n + 1u) {
        let light = lights[light_indices[cell.x + n]];
        let at = light.position_range;
        let to_light = at.xyz - in.world_position;
        let distance_to = length(to_light);
        let toward = to_light / max(distance_to, 1e-4);
        let reach = clamp(1.0 - distance_to / at.w, 0.0, 1.0);
        let facing = max(dot(normal, toward), 0.0);
        // A spot: full inside the cone, fading over its last tenth.
        let spot = light.spot;
        let along = dot(-toward, spot.xyz);
        let edge = spot.w + (1.0 - spot.w) * 0.1;
        let cone = select(smoothstep(spot.w, edge, along), 1.0, spot.w < -1.5);
        // A lamp's shadow, by a ray to it — only where it lights at all.
        // Or by its shadow map, where it has one.
        var blocked = 1.0;
        if reach * facing * cone > 0.0 && (flags & 4u) != 0u {
            if frame.ray.y > 0.5 {
                let start = in.world_position + geometric * 0.02;
                // Aimed at a point of the lamp's ball, turned each frame
                // and pixel: TAA gathers them into a soft shadow.
                var aim = at.xyz;
                let size = frame.ray_params.w;
                if size > 0.0 {
                    let turn = frame.ambient_occlusion.w;
                    let n1 = fract(pixel_noise(in.clip_position.xy) + turn);
                    let n2 = fract(pixel_noise(in.clip_position.yx + vec2<f32>(17.0, 5.0)) + turn * 1.7 + f32(n) * 0.37);
                    let side = basis_of(toward);
                    let a = n2 * 6.2831853;
                    aim = aim + (side.t * cos(a) + side.b * sin(a)) * sqrt(n1) * size;
                }
                let to_aim = aim - start;
                let aim_distance = length(to_aim);
                blocked = ray_visible(start, to_aim / max(aim_distance, 1e-4), max(aim_distance - size - 0.05, 0.0));
            } else {
                blocked = lamp_shadow(light, in.world_position, geometric, distance_to);
            }
        }
        color = color + direct(b, normal, toward, to_eye, highlights)
            * light.color_shadow.rgb * facing * reach * reach * cone * direct_ao * blocked;
    }

    // Hemisphere ambient: a face turned up sees sky, one turned down sees
    // bounce off the ground. A single constant here is what makes every
    // shaded surface in a scene the same dead colour.
    var sky_light = frame.sky_color.rgb;
    if frame.air.x > 0.5 {
        // The physical sky's light, with its direction: its picture round
        // this face, over its picture round a face turned up.
        let up = sky_toward(vec3<f32>(0.0, 1.0, 0.0));
        let here = sky_toward(normal);
        sky_light = sky_light * here / max(up, vec3<f32>(1e-4));
    }
    var all_round = mix(frame.ground_color.rgb, sky_light, normal.y * 0.5 + 0.5);
    if frame.ambient_equator.w > 0.5 {
        // The scene's three colours, blended as smoothly as Unity's
        // gradient is once it is spherical harmonics: straight up sees
        // only the sky, sideways half the horizon and a quarter each of
        // the sky and the ground.
        let y = clamp(normal.y, -1.0, 1.0);
        all_round = frame.sky_color.rgb * (1.0 + y) * (1.0 + y) * 0.25
            + frame.ground_color.rgb * (1.0 - y) * (1.0 - y) * 0.25
            + frame.ambient_equator.rgb * (1.0 - y * y) * 0.5;
    }
    var ambient = around(in.world_position, normal, all_round);
    // Inside an irradiance volume its probes give the diffuse light.
    let volume = ddgi_irradiance(in.world_position, normal, to_eye);
    let luminance = vec3<f32>(0.2126, 0.7152, 0.0722);
    let reaches = clamp(dot(volume.rgb, luminance) / max(dot(ambient, luminance), 1e-4), 0.0, 1.0);
    sky_share = mix(1.0, reaches, volume.a);
    ambient = mix(ambient, volume.rgb, volume.a);
    color = color + b.diffuse * (ambient * ao + bounce) * baked;
    if (flags & 2u) != 0u {
        let n_v = clamp(dot(normal, to_eye), 0.0, 1.0);
        let fresnel = pow(1.0 - n_v, 4.0);
        let reduction = 1.0 / (b.roughness2 + 1.0);
        let seen = reflected(in.world_position, reflect(-to_eye, normal), b.perceptual_roughness);
        color = color + seen * reduction * mix(b.specular, vec3<f32>(b.grazing), fresnel) * ao * baked;
    }
    let emission = shaped.emission;
    color = color + emission;

    let distance = length(in.world_position - frame.camera_position.xyz);
    color = mix(color, fog_color_towards(in.world_position - frame.camera_position.xyz), fog_amount(distance));

    // Glass by rays: what is behind it, bent through it, where the blend
    // would have laid the unbent picture — the glass's own light over it
    // as much as it is opaque.
    if frame.glass.x > 0.5 && alpha < 0.999 && unlit < 0.5 {
        let facing_eye = select(normal, -normal, dot(normal, to_eye) < 0.0);
        let through = ray_refraction(in.world_position, -to_eye, facing_eye, frame.glass.y);
        if through.a > 0.5 {
            color = color * alpha + through.rgb * albedo * (1.0 - alpha);
            alpha = 1.0;
        }
    }
    // An unlit surface takes neither the light nor the fog: it is not a
    // surface the sun falls on, it is something that emits. Selecting with a
    // mix rather than branching keeps both paths on the same instruction
    // stream, which matters because the two are interleaved in one draw.
    var out = mix(color, albedo + emission, unlit);
    // A dust wall between it and the eye (the cloud pass stopped there) —
    // unless it stands nearer than where the wall can begin: the clouds'
    // picture is a quarter of the frame, and a thin post in front of the
    // wall must not take the wall's colour from the texel it shares.
    // And dust devils and sand off the crests, marched with it.
    if (frame.weather[1].z > 0.0 && dust_can_reach(in.world_position)) || frame.dust.x > 0.5 {
        let c = textureSampleLevel(cloud_layer, fog_sampler, in.clip_position.xy / frame.cluster_depth.zw, 0.0);
        out = out * c.a + c.rgb;
    }
    // Nor the air's haze, which is sunlight too.
    out = mix(through_air(out, in.clip_position.xy, length(in.world_position - frame.camera_position.xyz)), out, unlit);
    out = through_fog(out, in.clip_position.xy, -dot(frame.view_depth, vec4<f32>(in.world_position, 1.0)));
    if (flags & 8u) != 0u {
        out = out * alpha;
    }
    return vec4<f32>(out, alpha);
}

/// The depth-and-normals prepass for ambient occlusion: the world normal of
/// what is solid, cut out where the surface is.
@fragment
fn fs_normals(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let alpha = in.surface.z * surface_at(in.maps, in.uv).a;
    if in.surface.w > 0.0 && alpha < in.surface.w {
        discard;
    }
    return vec4<f32>(normalize(in.normal) * select(-1.0, 1.0, front), 1.0);
}

struct Sand {
    normal: vec3<f32>,
    // what the albedo is times: drifting sand lightens it
    shade: f32,
    // a glinting grain's facet here, or zero
    glint: vec3<f32>,
};

/// The height of wind ripples at `u` along the wind and `v` across it, in
/// wavelengths: a gentle slope up the windward side and a short steep one
/// down the lee, the crests wandering and now and then forking.
fn ripple_height(u: f32, v: f32) -> f32 {
    let wander = cloud_noise(vec3<f32>(u * 0.18, v * 0.55, 0.0)) * 2.4
        + cloud_noise(vec3<f32>(u * 0.5, v * 1.7, 3.0)) * 0.6;
    let x = fract(u + wander);
    // Up for most of a wavelength, then down the lee.
    return select((1.0 - x) / 0.25, x / 0.75, x < 0.75) - 0.5;
}

/// Sand's surface at `p`: ripples laid square to the wind, faded where a
/// pixel spans more than a few of them (so far sand does not shimmer) and
/// on slopes too steep to hold them; streaks of drifting sand in a strong
/// wind; now and then a grain that glints.
fn sand_surface(p: vec3<f32>, normal: vec3<f32>, geometric: vec3<f32>, across: vec3<f32>, down: vec3<f32>, built: vec2<f32>) -> Sand {
    let wind = frame.foliage.wind;
    var w = vec2<f32>(wind.x, wind.y);
    if dot(w, w) < 1e-6 {
        w = vec2<f32>(1.0, 0.0);
    }
    w = normalize(w);
    let side = vec2<f32>(-w.y, w.x);
    let flat_enough = smoothstep(0.7, 0.93, geometric.y);
    let wavelength = 0.14;
    let u = dot(p.xz, w) / wavelength;
    let v = dot(p.xz, side) / wavelength;
    // How many wavelengths one pixel spans: past about a third, fade out.
    let footprint = max(length(across.xz), length(down.xz)) / wavelength;
    let fade = (1.0 - smoothstep(0.15, 0.45, footprint)) * flat_enough;
    var out: Sand;
    out.normal = normal;
    out.shade = 1.0;
    out.glint = vec3<f32>(0.0);
    let along = vec3<f32>(w.x, 0.0, w.y);
    let square = vec3<f32>(side.x, 0.0, side.y);
    var slope = vec3<f32>(0.0);
    let e = 0.05;
    if fade > 0.0 {
        let du = (ripple_height(u + e, v) - ripple_height(u - e, v)) / (2.0 * e);
        let dv = (ripple_height(u, v + e) - ripple_height(u, v - e)) / (2.0 * e);
        // A centimetre high in fourteen: slope per metre.
        slope += (along * du + square * dv) * (0.012 / wavelength) * fade * (1.0 - built.x);
    }
    // And the bigger ripples they ride on, which still show further off.
    let big = 0.6;
    let far_fade = (1.0 - smoothstep(0.15, 0.45, footprint * wavelength / big)) * flat_enough;
    if far_fade > 0.0 {
        let bu = dot(p.xz, w) / big + 7.3;
        let bv = dot(p.xz, side) / big;
        let du = (ripple_height(bu + e, bv) - ripple_height(bu - e, bv)) / (2.0 * e);
        let dv = (ripple_height(bu, bv + e) - ripple_height(bu, bv - e)) / (2.0 * e);
        slope += (along * du + square * dv) * (0.025 / big) * far_fade * (1.0 - built.y);
    }
    out.normal = normalize(normal - slope);
    // Drifting: in a wind past a stiff breeze, sand running along the
    // ground in streaks, lighter than what lies still.
    let gale = clamp((wind.z - 1.2) / 1.5, 0.0, 1.0) + frame.weather[1].y;
    if gale > 0.0 {
        let run = vec3<f32>(dot(p.xz, w) * 0.9 - wind.w * 4.0 * max(wind.z, 1.0), dot(p.xz, side) * 6.0, wind.w * 0.7);
        let streak = smoothstep(0.55, 0.85, cloud_noise(run) * 0.7 + cloud_noise(run * 2.7) * 0.3);
        out.shade = 1.0 + 0.18 * streak * clamp(gale, 0.0, 1.0) * flat_enough;
    }
    // Glints: one grain in a few dozen, each its own random facet, in
    // cells of a centimetre and a half — only while a cell is bigger than
    // the pixel, or they would crawl.
    let cells = 1.0 / 0.015;
    let cell_footprint = max(length(across), length(down)) * cells;
    if cell_footprint < 1.2 {
        let cell = floor(p * cells);
        let h = cloud_hash(cell);
        if h > 0.965 {
            let tilt = vec3<f32>(cloud_hash(cell + 17.0), cloud_hash(cell + 31.0), cloud_hash(cell + 53.0)) * 2.0 - 1.0;
            out.glint = normalize(out.normal + tilt * 0.8) * (1.0 - smoothstep(0.6, 1.2, cell_footprint));
        }
    }
    return out;
}

/// A footprint's height across it, in its depth: −1 pressed right in, up
/// to a third of that pushed up in a rim round the edge. `f` is across
/// and along the foot, −1 to 1, toes at −1: a sole and a heel run
/// together.
fn footprint_height(f: vec2<f32>) -> f32 {
    let sole = length(vec2<f32>((f.x - 0.06) / 0.86, (f.y + 0.3) / 0.6));
    let heel = length(vec2<f32>(f.x / 0.64, (f.y - 0.5) / 0.42));
    // A smooth union: the arch between them filled in, narrower.
    let k = 0.45;
    let blend = clamp(0.5 + 0.5 * (heel - sole) / k, 0.0, 1.0);
    let s = mix(heel, sole, blend) - k * blend * (1.0 - blend);
    let pit = 1.0 - smoothstep(0.6, 1.0, s);
    let rim = exp(-pow((s - 1.12) / 0.16, 2.0));
    return -pit + 0.35 * rim;
}

/// The greybox surface: a line every metre and alternate metres a shade
/// apart, projected along the face's main axis so walls, floors and
/// ceilings all show metres whatever the object's scale. Line width comes
/// from the screen-space derivative, so a line stays a line at any distance
/// instead of shimmering into moiré.
fn metre_grid(world_position: vec3<f32>, normal: vec3<f32>) -> f32 {
    let n = abs(normal);
    let facing_x = n.x > n.y && n.x > n.z;
    let facing_z = !facing_x && n.z > n.y;
    let plane = select(
        select(world_position.xz, world_position.xy, facing_z),
        world_position.zy,
        facing_x,
    );
    let width = max(fwidth(plane), vec2<f32>(1e-4));
    let to_line = abs(fract(plane + 0.5) - 0.5) / width;
    let line = 1.0 - min(min(to_line.x, to_line.y), 1.0);
    let cell = floor(plane);
    let checker = abs(cell.x + cell.y) % 2.0;
    return mix(1.0, 0.88, checker) * mix(1.0, 0.45, line);
}

/// How much fog stands between the eye and a point this far away.
fn fog_amount(distance: f32) -> f32 {
    let mode = u32(frame.fog_range.z + 0.5);
    let density = frame.fog_range.w;
    if mode == 1u {
        return 1.0 - exp(-density * distance);
    }
    if mode == 2u {
        let d = density * distance;
        return 1.0 - exp(-d * d);
    }
    let span = max(frame.fog_range.y - frame.fog_range.x, 0.001);
    return clamp((distance - frame.fog_range.x) / span, 0.0, 1.0);
}

struct SkyOut {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

/// Rain streaks or snowflakes in one layer of the air round the camera:
/// cells over the directions the camera looks, each holding a drop or not,
/// falling with time and slanted by the wind.
fn falling_layer(azimuth: f32, elevation: f32, layer: f32, t: f32, amount: f32, snow: bool) -> f32 {
    let columns = select(70.0 + layer * 55.0, 40.0 + layer * 30.0, snow);
    let rows = select(columns * 0.12, columns * 0.9, snow);
    var p = vec2<f32>(azimuth / 6.2831853 * columns, elevation / 3.14159 * rows);
    let speed = select(9.0 + layer * 3.0, 0.9 + layer * 0.3, snow);
    p.y += t * speed;
    // Slanted by the wind's side-on share.
    p.x += p.y * frame.foliage.wind.x * frame.foliage.wind.z * select(0.06, 0.2, snow);
    let cell = floor(p);
    let f = fract(p);
    if hash21(cell + layer * 17.0) > amount * select(0.45, 0.35, snow) {
        return 0.0;
    }
    let x = hash21(cell + 2.3) * 0.8 + 0.1;
    if snow {
        let sway = sin(t * 1.3 + cell.y * 2.1 + layer) * 0.15;
        let c = vec2<f32>(x + sway, hash21(cell + 4.7) * 0.8 + 0.1);
        let r = 0.08 / (1.0 + layer * 0.5);
        return smoothstep(r, r * 0.3, length((f - c) * vec2<f32>(1.0, rows / columns * 1.1)));
    }
    let width = 0.04 / (1.0 + layer * 0.4);
    return smoothstep(width, 0.0, abs(f.x - x)) * smoothstep(0.0, 0.35, f.y) * smoothstep(1.0, 0.65, f.y);
}

/// Sand flying past on the wind: short streaks along it, fast, low down
/// thicker than high up.
fn flying_sand(azimuth: f32, elevation: f32, d: vec3<f32>, layer: f32, t: f32, amount: f32) -> f32 {
    let columns = 90.0 + layer * 60.0;
    let rows = columns * 0.5;
    var p = vec2<f32>(azimuth / 6.2831853 * columns, elevation / 3.14159 * rows);
    // Across the view the way the wind blows past it.
    let wind = vec2<f32>(frame.foliage.wind.x, frame.foliage.wind.y);
    let right = normalize(vec2<f32>(-d.z, d.x) + vec2<f32>(1e-5));
    let across = dot(wind, right) * max(frame.foliage.wind.z, 0.5);
    p.x -= t * across * (14.0 + layer * 6.0);
    p.y += sin(t * 3.0 + floor(p.x) * 0.7) * 0.15;
    let cell = floor(p);
    let f = fract(p);
    let low = 1.0 - smoothstep(-0.1, 0.35, d.y);
    if hash21(cell + layer * 31.0) > amount * 0.32 * (0.3 + low) {
        return 0.0;
    }
    let y = hash21(cell + 4.1) * 0.8 + 0.1;
    let x = hash21(cell + 7.3);
    let along = abs(f.x - x);
    return smoothstep(0.04, 0.0, abs(f.y - y)) * smoothstep(0.3, 0.0, along) * 0.7;
}

// Rain and snow falling, over the finished frame: faint streaks and flakes
// lit by the sky, a touch of the sun, a little dimming where they cover.
@fragment
fn fs_precipitation(in: SkyOut) -> @location(0) vec4<f32> {
    let near = frame.inverse_view_projection * vec4<f32>(in.ndc, 0.0, 1.0);
    let far = frame.inverse_view_projection * vec4<f32>(in.ndc, 1.0, 1.0);
    let d = normalize(far.xyz / far.w - near.xyz / near.w);
    let azimuth = atan2(d.x, d.z);
    let elevation = asin(clamp(d.y, -1.0, 1.0));
    let t = frame.foliage.wind.w;
    let rain = frame.weather[0].w;
    let snowfall = frame.weather[1].x;
    let sandstorm = frame.weather[1].y;
    var cover = 0.0;
    var snow_cover = 0.0;
    var sand_cover = 0.0;
    for (var layer = 0; layer < 3; layer = layer + 1) {
        let l = f32(layer);
        if rain > 0.0 {
            cover += falling_layer(azimuth, elevation, l, t, rain, false) * (0.35 / (1.0 + l));
        }
        if snowfall > 0.0 {
            snow_cover += falling_layer(azimuth, elevation, l, t, snowfall, true) * (0.9 / (1.0 + l * 0.6));
        }
        if sandstorm > 0.0 {
            sand_cover += flying_sand(azimuth, elevation, d, l, t, sandstorm) * (0.5 / (1.0 + l * 0.5));
        }
    }
    let light = frame.sky_color.rgb * 1.5 + frame.sun_color.rgb * 0.15;
    let sand = vec3<f32>(0.95, 0.6, 0.3) * (frame.sky_color.rgb + frame.sun_color.rgb * 0.5) * 1.3;
    let rgb = light * cover + (light * 0.6 + vec3<f32>(0.25)) * snow_cover + sand * sand_cover;
    return vec4<f32>(rgb, clamp(cover * 0.3 + snow_cover * 0.8 + sand_cover * 0.8, 0.0, 1.0));
}

/// How deep a point lies under the water above it, if any, in metres.
fn under_water(p: vec3<f32>) -> f32 {
    var depth = 0.0;
    for (var i = 0u; i < 4u; i = i + 1u) {
        let top = frame.waters[i * 2u];
        let area = frame.waters[i * 2u + 1u];
        if top.y > 0.5 && p.y < top.x && p.x > area.x && p.z > area.y && p.x < area.z && p.z < area.w {
            depth = max(depth, top.x - p.y);
        }
    }
    return depth;
}

/// Caustics: the sun's light gathered into dancing lines by the waves
/// above — a warped web, brighter where its strands cross.
fn caustics(p: vec2<f32>, t: f32) -> f32 {
    var q = p * 1.8;
    for (var i = 0; i < 3; i = i + 1) {
        let fi = f32(i);
        q = q + vec2<f32>(sin(q.y * 1.3 + t * 0.9 + fi * 1.7), sin(q.x * 1.1 - t * 0.8 + fi * 2.3)) * 0.45;
    }
    let lines = abs(sin(q.x) * sin(q.y));
    return pow(1.0 - lines, 6.0);
}

/// The water's surface: a few long waves running with the wind and
/// shorter ones across them, as a slope; `height` scales them all.
fn water_normal(p: vec2<f32>, t: f32, height: f32) -> vec3<f32> {
    let wind = select(vec2<f32>(1.0, 0.0), normalize(frame.foliage.wind.xy), length(frame.foliage.wind.xy) > 0.0);
    var slope = vec2<f32>(0.0);
    for (var i = 0; i < 5; i = i + 1) {
        let fi = f32(i);
        let turn = (fi - 2.0) * 0.7 + sin(fi * 2.4) * 0.3;
        let d = vec2<f32>(wind.x * cos(turn) - wind.y * sin(turn), wind.x * sin(turn) + wind.y * cos(turn));
        let k = 1.1 * pow(1.83, fi);
        let a = 0.03 / pow(1.8, fi);
        let phase = dot(d, p) * k - t * sqrt(9.8 * k);
        slope += d * (a * k * cos(phase));
    }
    // Fine ripples on top.
    let fine = vec2<f32>(value_noise(p * 6.0 + t * 0.7), value_noise(p * 6.0 - t * 0.6 + 7.3)) - 0.5;
    slope = (slope + fine * 0.025) * height;
    return normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
}

// Water: waves reflecting the sky and what is round, the colour of the
// depth beneath, foam at the shore. What is under it shows through: this
// is blended, premultiplied, over the solid scene already drawn, and how
// much of that shows is how deep the water is there — read from the
// prepass's depth.
@fragment
fn fs_water(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = frame.foliage.wind.w;
    let p = in.world_position;
    let waves = max(in.detail.z, 0.0) * max(frame.foliage.wind.z, 0.2);
    let n = water_normal(p.xz, t, waves);
    let to_eye = normalize(frame.camera_position.xyz - p);
    let to_sun = -normalize(frame.sun_direction.xyz);

    // How deep it is here: from the surface down to what the solid scene
    // has under this pixel, along the view.
    let d = textureLoad(scene_depth, vec2<i32>(in.clip_position.xy), 0);
    let near = frame.cluster_depth.x;
    let far = near * exp(frame.cluster_depth.y);
    let behind = near * far / (far - d * (far - near));
    let here = -dot(frame.view_depth, vec4<f32>(p, 1.0));
    let below = max(behind - here, 0.0) * max(to_eye.y, 0.2);
    let clarity = max(in.surface.x, 0.05);
    let murk = 1.0 - exp(-below / clarity);

    let fresnel = 0.02 + 0.98 * pow(1.0 - max(dot(n, to_eye), 0.0), 5.0);
    let shadow = sunlight(p, vec3<f32>(0.0, 1.0, 0.0));
    let body = in.base_color * (frame.sky_color.rgb + frame.sun_color.rgb * max(to_sun.y, 0.0) * 0.35 * shadow);
    let mirrored = reflected(p, reflect(-to_eye, n), 0.03);
    let glint = pow(max(dot(n, normalize(to_sun + to_eye)), 0.0), 600.0) * 30.0 * frame.sun_color.rgb * shadow;

    var rgb = body * murk * (1.0 - fresnel) + mirrored * fresnel + glint;
    var alpha = clamp(murk * (1.0 - fresnel) + fresnel, 0.0, 1.0);
    // By rays: the bottom seen through the waves, bent at the surface, with
    // the depth's colour over it — in place of the unbent picture behind.
    if frame.glass.x > 0.5 {
        let through = ray_refraction(p, -to_eye, n, 1.33);
        if through.a > 0.5 {
            rgb = mix(through.rgb, body, murk) * (1.0 - fresnel) + mirrored * fresnel + glint;
            alpha = 1.0;
        }
    }

    // Foam where it is shallow, broken up.
    let lace = value_noise(p.xz * 2.2 + vec2<f32>(t * 0.25, -t * 0.2)) * 0.6
        + value_noise(p.xz * 6.5 - vec2<f32>(t * 0.3, t * 0.1)) * 0.4;
    let edge = 1.0 - smoothstep(0.0, 0.22, below);
    let foam = in.surface.y * edge * smoothstep(0.55 - edge * 0.35, 0.8 - edge * 0.2, lace);
    let foam_light = (frame.sky_color.rgb + frame.sun_color.rgb * max(to_sun.y, 0.0) * shadow) * 0.85;
    rgb = mix(rgb, foam_light, foam);
    alpha = max(alpha, foam);

    // Through the air and the fog in front of it, as a premultiplied colour:
    // what they add counts only as much as the water covers.
    let distance = length(p - frame.camera_position.xyz);
    let air0 = through_air(vec3<f32>(0.0), in.clip_position.xy, distance);
    let air1 = through_air(vec3<f32>(1.0), in.clip_position.xy, distance) - air0;
    rgb = rgb * air1 + air0 * alpha;
    let fog0 = through_fog(vec3<f32>(0.0), in.clip_position.xy, here);
    let fog1 = through_fog(vec3<f32>(1.0), in.clip_position.xy, here) - fog0;
    rgb = rgb * fog1 + fog0 * alpha;
    return vec4<f32>(rgb, alpha);
}

/// One triangle over the screen, on the far plane.
@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    var out: SkyOut;
    out.position = vec4<f32>(x, y, 1.0, 1.0);
    out.ndc = vec2<f32>(x, y);
    return out;
}

/// The night sky that way: stars — a few in a thousand cells of a cube
/// round the eye, each its own brightness and warmth, twinkling a little —
/// and the Milky Way, a band round a great circle, lumpy, with dark lanes
/// of dust down its middle. Faint toward the horizon, where the air is
/// thick.
fn night_sky(d: vec3<f32>) -> vec3<f32> {
    let a = abs(d);
    var uv: vec2<f32>;
    var face: f32;
    if a.x >= a.y && a.x >= a.z {
        uv = d.yz / a.x;
        face = select(0.0, 1.0, d.x < 0.0);
    } else if a.y >= a.z {
        uv = d.xz / a.y;
        face = select(2.0, 3.0, d.y < 0.0);
    } else {
        uv = d.xy / a.z;
        face = select(4.0, 5.0, d.z < 0.0);
    }
    let g = uv * 220.0;
    let cell = floor(g);
    let key = vec3<f32>(cell, face * 17.0);
    let h = cloud_hash(key);
    var light = vec3<f32>(0.0);
    if h > 0.972 {
        let at = vec2<f32>(cloud_hash(key + 3.1), cloud_hash(key + 7.7)) * 0.7 + 0.15;
        let off = fract(g) - at;
        let bright = pow(cloud_hash(key + 11.3), 7.0) * 7.0 + 0.25;
        let twinkle = 0.75 + 0.25 * sin(frame.foliage.wind.w * (2.0 + h * 6.0) + h * 91.0);
        let warmth = mix(vec3<f32>(0.72, 0.84, 1.0), vec3<f32>(1.0, 0.86, 0.68), cloud_hash(key + 5.0));
        light = warmth * bright * twinkle * exp(-dot(off, off) * 160.0) * 0.4;
    }
    let across = normalize(vec3<f32>(0.35, 0.55, -0.76));
    let b = dot(d, across);
    let band = exp(-b * b / 0.045);
    let lumps = cloud_noise(d * 9.0) * 0.6 + cloud_noise(d * 23.0) * 0.4;
    let lanes = 1.0 - 0.6 * smoothstep(0.5, 0.75, cloud_noise(d * 14.0 + 5.0)) * exp(-b * b / 0.006);
    // The Milky Way: a crowd of faint stars, thick in the band and thinned
    // by its dust lanes, over a glow of the ones too faint to tell apart —
    // grain, not smoke. It shows only once the sky is truly dark.
    let fine = uv * 640.0;
    let fine_key = vec3<f32>(floor(fine), face * 29.0 + 3.0);
    var crowd = 0.0;
    if cloud_hash(fine_key) > 1.0 - 0.3 * band * lanes {
        let at = vec2<f32>(cloud_hash(fine_key + 2.3), cloud_hash(fine_key + 4.1)) * 0.6 + 0.2;
        let off = fract(fine) - at;
        crowd = exp(-dot(off, off) * 60.0) * (0.3 + cloud_hash(fine_key + 8.9)) * 0.1;
    }
    let glow = band * (0.35 + 0.65 * lumps) * lanes * 0.045;
    let dark = frame.night.x * frame.night.x;
    light += vec3<f32>(0.8, 0.83, 1.0) * (crowd + glow) * dark;
    return light * smoothstep(0.0, 0.25, d.y);
}

/// A stroke of lightning seen along `direction` from `eye`: a white-hot
/// core a pixel or two wide, however far, and a glow round it.
fn lightning_channel(eye: vec3<f32>, direction: vec3<f32>) -> vec3<f32> {
    let count = u32(frame.glass.z);
    // How wide a pixel is, as an angle.
    let pixel = 2.0 / max(frame.cluster_depth.w, 1.0);
    var core = 0.0;
    var halo = 0.0;
    for (var i = 1u; i < count; i = i + 1u) {
        let b = frame.bolt[i];
        if b.w <= 0.0 {
            continue;
        }
        let a = frame.bolt[i - 1u].xyz;
        // The nearest points of the view's ray and the segment.
        let u = b.xyz - a;
        let w0 = eye - a;
        let dd = dot(direction, direction);
        let du = dot(direction, u);
        let uu = dot(u, u);
        let dw = dot(direction, w0);
        let uw = dot(u, w0);
        let den = max(dd * uu - du * du, 1e-6);
        let s = clamp((dd * uw - du * dw) / den, 0.0, 1.0);
        let t = max((du * s - dw) / dd, 1.0);
        let gap = length(eye + direction * t - (a + u * s));
        let angle = gap / t;
        let width = max(0.8 / t, pixel * 0.8);
        core = max(core, exp(-(angle / width) * (angle / width)) * b.w);
        halo = max(halo, exp(-angle / (width * 14.0)) * b.w);
    }
    return vec3<f32>(0.85, 0.9, 1.0) * (core * 40.0 + halo * 1.5);
}

/// URP's procedural skybox, simply: the horizon's colour rising into the
/// zenith's, the ground below, and the sun — a disc far brighter than white,
/// with a glow around it — where the light comes from.
@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    if frame.sky_zenith.w < 0.5 {
        // A plain colour, drawn only for the fog in front of it.
        return vec4<f32>(through_fog(frame.clear_color.rgb, in.position.xy, frame.volume.y), 1.0);
    }
    let near = frame.inverse_view_projection * vec4<f32>(in.ndc, 0.0, 1.0);
    let far = frame.inverse_view_projection * vec4<f32>(in.ndc, 1.0, 1.0);
    let direction = normalize(far.xyz / far.w - near.xyz / near.w);
    let up = direction.y;
    var color: vec3<f32>;
    if frame.sky_zenith.w > 1.5 {
        color = physical_sky(direction);
    } else if up >= 0.0 {
        color = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, pow(up, 0.45));
    } else {
        color = mix(frame.sky_horizon.rgb, frame.sky_ground.rgb, pow(-up, 0.3));
    }
    // The stars, as night falls; and the moon's light in the air, a deep
    // blue, paler toward the horizon.
    if frame.night.x > 0.0 {
        let glow = frame.sky_color.rgb * (0.6 + 1.0 * (1.0 - clamp(up, 0.0, 1.0)));
        color += (night_sky(direction) + glow) * frame.night.x;
    }
    // Lightning: the sky flares, and the channel stands in it.
    let flash = frame.glass.w;
    if flash > 0.0 {
        color += vec3<f32>(0.45, 0.5, 0.65) * flash * (0.6 + 0.4 * clamp(up, 0.0, 1.0));
        color += lightning_channel(frame.camera_position.xyz, direction) * min(flash, 1.0);
    }
    let to_sun = -normalize(frame.sun_direction.xyz);
    let facing = dot(direction, to_sun);
    let radius = frame.sky_horizon.w;
    let disc = smoothstep(radius, radius + (1.0 - radius) * 0.15, facing);
    let glow = pow(max(facing, 0.0), 256.0) * 0.6 + pow(max(facing, 0.0), 16.0) * 0.08;
    let sun = frame.sun_color.rgb * (disc * 20.0 * step(radius, 0.99999) + glow) * step(0.0, up + 0.02);
    var sky = (color + sun) * frame.sky_ground.w;
    if frame.clouds[0].x > 0.0 || frame.weather[1].z > 0.0 || frame.dust.x > 0.5 {
        let c = textureSampleLevel(cloud_layer, fog_sampler, in.position.xy / frame.cluster_depth.zw, 0.0);
        sky = sky * c.a + c.rgb;
    }
    return vec4<f32>(through_fog(sky, in.position.xy, frame.volume.y), 1.0);
}
