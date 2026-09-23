// The scene's lighting: one sun, hemisphere ambient, point and spot lights,
// distance fog, and the sky behind everything. Drawn in linear light into a
// high-dynamic-range buffer; post.wgsl turns that into a picture.

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
    // Lights, three vectors each: position and range; colour; spot
    // direction and the cosine of half its cone (-2: every way).
    lights: array<vec4<f32>, 24>,
    // How many are on, in x.
    light_count: vec4<f32>,
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
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_map: texture_depth_2d_array;
// A comparison sampler: the hardware does the depth test and the bilinear
// filter in one fetch, so every tap is already a 2x2 average.
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
// Ambient occlusion, a texel per pixel (ssao.wgsl).
@group(0) @binding(4) var occlusion: texture_2d<f32>;

// The shadow pass's one matrix: the cascade being drawn. Beside the frame
// at binding 3, in the shadow pass's own group.
struct Caster {
    view_projection: mat4x4<f32>,
};
@group(0) @binding(3) var<uniform> caster: Caster;

// The surface's own image. Every draw binds one; an untextured material
// binds a single white pixel, so the shader never needs a branch and an
// untextured surface is its colour times one.
@group(1) @binding(0) var surface_texture: texture_2d<f32>;
@group(1) @binding(1) var surface_sampler: sampler;

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
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) base_color: vec3<f32>,
    // 0 lit, 1 unlit, 2 lit with the metre grid.
    @location(3) shading: f32,
    @location(4) uv: vec2<f32>,
    @location(5) surface: vec4<f32>,
    @location(6) emission: vec4<f32>,
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
    out.uv = in.uv;
    out.surface = in.surface;
    out.emission = in.emission;
    return out;
}

/// The depth-only pass, seen from the sun, one cascade at a time.
@vertex
fn vs_shadow(in: VertexInput) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    return caster.view_projection * model * vec4<f32>(in.position, 1.0);
}

struct ClipOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // alpha, threshold
    @location(1) alpha: vec2<f32>,
};

/// The same, for what is cut out by its alpha: a leaf's shadow is a leaf.
@vertex
fn vs_shadow_clip(in: VertexInput) -> ClipOut {
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    var out: ClipOut;
    out.position = caster.view_projection * model * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.alpha = in.surface.zw;
    return out;
}

@fragment
fn fs_shadow_clip(in: ClipOut) {
    if in.alpha.x * textureSample(surface_texture, surface_sampler, in.uv).a < in.alpha.y {
        discard;
    }
}

/// How much sun reaches a point: 1.0 in the open, 0.0 in full shadow.
///
/// From the first cascade whose sphere holds the point — the finest one
/// that covers it — and fading out over the last tenth of the last one, so
/// the shadow distance is not a line on the ground.
fn sunlight(world_position: vec3<f32>, normal: vec3<f32>) -> f32 {
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

@vertex
fn vs(in: VertexInput) -> VertexOutput {
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    let world = model * vec4<f32>(in.position, 1.0);

    var out: VertexOutput;
    out.clip_position = frame.view_projection * world;
    out.world_position = world.xyz;
    // Assumes uniform scale. A non-uniform one would need the inverse
    // transpose; scenes that want it can scale at import instead, which is
    // where scale is settled anyway.
    out.normal = (model * vec4<f32>(in.normal, 0.0)).xyz;
    out.base_color = in.color_and_shading.rgb;
    out.shading = in.color_and_shading.w;
    out.uv = in.uv;
    out.surface = in.surface;
    out.emission = in.emission;
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
    var sky: vec3<f32>;
    if direction.y >= 0.0 {
        sky = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, pow(direction.y, 0.45));
    } else {
        sky = mix(frame.sky_horizon.rgb, frame.sky_ground.rgb, pow(-direction.y, 0.3));
    }
    return mix(sky * frame.sky_ground.w, hemisphere, perceptual_roughness);
}

@fragment
fn fs(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let sampled = textureSample(surface_texture, surface_sampler, in.uv);
    let alpha = in.surface.z * sampled.a;
    // Alpha clipping: what is less opaque than the threshold is not drawn
    // at all.
    if in.surface.w > 0.0 && alpha < in.surface.w {
        discard;
    }
    // A face seen from behind — a two-sided leaf — is lit from its own side.
    let normal = normalize(in.normal) * select(-1.0, 1.0, front);
    let flags = u32(in.emission.w + 0.5);
    let unlit = f32(in.shading > 0.5 && in.shading < 1.5);
    let grid = f32(in.shading > 1.5);

    let albedo = in.base_color * sampled.rgb * mix(1.0, metre_grid(in.world_position, normal), grid);
    let to_eye = normalize(frame.camera_position.xyz - in.world_position);
    let b = brdf(albedo, in.surface.x, in.surface.y);
    let highlights = (flags & 1u) != 0u;

    let to_sun = -normalize(frame.sun_direction.xyz);
    var shadow = 1.0;
    if (flags & 4u) != 0u {
        shadow = sunlight(in.world_position, normal);
    }
    // Ambient occlusion darkens the light from all around, and a share of
    // the direct light too (URP's Direct Lighting Strength).
    var ao = 1.0;
    if frame.ambient_occlusion.x > 0.5 && unlit < 0.5 {
        ao = textureLoad(occlusion, vec2<i32>(in.clip_position.xy), 0).r;
    }
    let direct_ao = mix(1.0, ao, frame.ambient_occlusion.y);
    var color = direct(b, normal, to_sun, to_eye, highlights)
        * frame.sun_color.rgb * max(dot(normal, to_sun), 0.0) * shadow * direct_ao;

    // Point and spot lights: facing it, and fading to nothing at its range
    // — squared, so the edge of the pool is soft rather than a ring.
    let count = u32(frame.light_count.x);
    for (var i = 0u; i < count; i = i + 1u) {
        let at = frame.lights[i * 3u];
        let to_light = at.xyz - in.world_position;
        let distance_to = length(to_light);
        let toward = to_light / max(distance_to, 1e-4);
        let reach = clamp(1.0 - distance_to / at.w, 0.0, 1.0);
        let facing = max(dot(normal, toward), 0.0);
        // A spot: full inside the cone, fading over its last tenth.
        let spot = frame.lights[i * 3u + 2u];
        let along = dot(-toward, spot.xyz);
        let edge = spot.w + (1.0 - spot.w) * 0.1;
        let cone = select(smoothstep(spot.w, edge, along), 1.0, spot.w < -1.5);
        color = color + direct(b, normal, toward, to_eye, highlights)
            * frame.lights[i * 3u + 1u].rgb * facing * reach * reach * cone * direct_ao;
    }

    // Hemisphere ambient: a face turned up sees sky, one turned down sees
    // bounce off the ground. A single constant here is what makes every
    // shaded surface in a scene the same dead colour.
    let ambient = mix(frame.ground_color.rgb, frame.sky_color.rgb, normal.y * 0.5 + 0.5);
    color = color + b.diffuse * ambient * ao;
    if (flags & 2u) != 0u {
        let n_v = clamp(dot(normal, to_eye), 0.0, 1.0);
        let fresnel = pow(1.0 - n_v, 4.0);
        let reduction = 1.0 / (b.roughness2 + 1.0);
        let reflected = environment(reflect(-to_eye, normal), b.perceptual_roughness);
        color = color + reflected * reduction * mix(b.specular, vec3<f32>(b.grazing), fresnel) * ao;
    }
    color = color + in.emission.rgb;

    let distance = length(in.world_position - frame.camera_position.xyz);
    color = mix(color, frame.fog_color.rgb, fog_amount(distance));

    // An unlit surface takes neither the light nor the fog: it is not a
    // surface the sun falls on, it is something that emits. Selecting with a
    // mix rather than branching keeps both paths on the same instruction
    // stream, which matters because the two are interleaved in one draw.
    var out = mix(color, albedo + in.emission.rgb, unlit);
    if (flags & 8u) != 0u {
        out = out * alpha;
    }
    return vec4<f32>(out, alpha);
}

/// The depth-and-normals prepass for ambient occlusion: the world normal of
/// what is solid, cut out where the surface is.
@fragment
fn fs_normals(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let alpha = in.surface.z * textureSample(surface_texture, surface_sampler, in.uv).a;
    if in.surface.w > 0.0 && alpha < in.surface.w {
        discard;
    }
    return vec4<f32>(normalize(in.normal) * select(-1.0, 1.0, front), 1.0);
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

/// URP's procedural skybox, simply: the horizon's colour rising into the
/// zenith's, the ground below, and the sun — a disc far brighter than white,
/// with a glow around it — where the light comes from.
@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let near = frame.inverse_view_projection * vec4<f32>(in.ndc, 0.0, 1.0);
    let far = frame.inverse_view_projection * vec4<f32>(in.ndc, 1.0, 1.0);
    let direction = normalize(far.xyz / far.w - near.xyz / near.w);
    let up = direction.y;
    var color: vec3<f32>;
    if up >= 0.0 {
        color = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, pow(up, 0.45));
    } else {
        color = mix(frame.sky_horizon.rgb, frame.sky_ground.rgb, pow(-up, 0.3));
    }
    let to_sun = -normalize(frame.sun_direction.xyz);
    let facing = dot(direction, to_sun);
    let radius = frame.sky_horizon.w;
    let disc = smoothstep(radius, radius + (1.0 - radius) * 0.15, facing);
    let glow = pow(max(facing, 0.0), 256.0) * 0.6 + pow(max(facing, 0.0), 16.0) * 0.08;
    let sun = frame.sun_color.rgb * (disc * 20.0 * step(radius, 0.99999) + glow) * step(0.0, up + 0.02);
    return vec4<f32>((color + sun) * frame.sky_ground.w, 1.0);
}
