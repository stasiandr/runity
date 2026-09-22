// The valley's whole lighting model: one sun, hemisphere ambient, distance
// fog. See `docs/design/07-look.md` — flat shading, no specular, colour
// instead of material.

struct Frame {
    view_projection: mat4x4<f32>,
    // Direction the light travels: from the sun toward the ground.
    sun_direction: vec4<f32>,
    // Already multiplied by intensity on the CPU.
    sun_color: vec4<f32>,
    sky_color: vec4<f32>,
    ground_color: vec4<f32>,
    fog_color: vec4<f32>,
    // start, end, unused, unused
    fog_range: vec4<f32>,
    camera_position: vec4<f32>,
    light_view_projection: mat4x4<f32>,
    // depth bias, normal offset in world units, one texel in UV, on/off
    shadow_params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
// A comparison sampler: the hardware does the depth test and the bilinear
// filter in one fetch, so every tap is already a 2x2 average.
@group(0) @binding(2) var shadow_sampler: sampler_comparison;

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
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) base_color: vec3<f32>,
    @location(3) unlit: f32,
    @location(4) uv: vec2<f32>,
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
    out.unlit = in.color_and_shading.w;
    out.uv = in.uv;
    return out;
}

/// The depth-only pass, seen from the sun.
@vertex
fn vs_shadow(in: VertexInput) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    return frame.light_view_projection * model * vec4<f32>(in.position, 1.0);
}

/// How much sun reaches a point: 1.0 in the open, 0.0 in full shadow.
fn sunlight(world_position: vec3<f32>, normal: vec3<f32>) -> f32 {
    if frame.shadow_params.w < 0.5 {
        return 1.0;
    }

    // Offsetting along the normal before the lookup is what handles grazing
    // angles: there the depth error grows with the slope, and no constant
    // bias large enough to cover it is small enough to keep contact.
    let offset = world_position + normal * frame.shadow_params.y;
    let light_clip = frame.light_view_projection * vec4<f32>(offset, 1.0);
    let ndc = light_clip.xyz / light_clip.w;

    // Clip space is -1..1 across and 0..1 deep; the map is indexed 0..1 with
    // v running the other way.
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if ndc.z > 1.0 || uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        // Outside the map is lit, not shadowed. The opposite choice makes
        // everything beyond the fitted frustum go black, which reads as a
        // wall of darkness at the edge of the scene.
        return 1.0;
    }

    let reference = ndc.z - frame.shadow_params.x;
    let texel = frame.shadow_params.z;
    // Nine taps, each of them already a hardware 2x2, so the edge is soft
    // enough that the map's resolution stops being visible as stairs.
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let tap = uv + vec2<f32>(f32(x), f32(y)) * texel;
            sum = sum + textureSampleCompare(shadow_map, shadow_sampler, tap, reference);
        }
    }
    return sum / 9.0;
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
    out.unlit = in.color_and_shading.w;
    out.uv = in.uv;
    return out;
}

@fragment
fn fs(in: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(in.normal);
    let to_sun = -normalize(frame.sun_direction.xyz);
    let lambert = max(dot(normal, to_sun), 0.0) * sunlight(in.world_position, normal);

    // Hemisphere ambient: a face turned up sees sky, one turned down sees
    // bounce off the ground. A single constant here is what makes every
    // shaded surface in a scene the same dead colour.
    let sky_amount = normal.y * 0.5 + 0.5;
    let ambient = mix(frame.ground_color.rgb, frame.sky_color.rgb, sky_amount);

    let sampled = textureSample(surface_texture, surface_sampler, in.uv);
    let albedo = in.base_color * sampled.rgb;
    var color = albedo * (ambient + frame.sun_color.rgb * lambert);

    let distance = length(in.world_position - frame.camera_position.xyz);
    let span = max(frame.fog_range.y - frame.fog_range.x, 0.001);
    let fog = clamp((distance - frame.fog_range.x) / span, 0.0, 1.0);
    color = mix(color, frame.fog_color.rgb, fog);

    // An unlit surface takes neither the light nor the fog: it is not a
    // surface the sun falls on, it is something that emits. Selecting with a
    // mix rather than branching keeps both paths on the same instruction
    // stream, which matters because the two are interleaved in one draw.
    return vec4<f32>(mix(color, albedo, in.unlit), 1.0);
}
