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
};

@group(0) @binding(0) var<uniform> frame: Frame;

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
};

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
    return out;
}

@fragment
fn fs(in: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(in.normal);
    let to_sun = -normalize(frame.sun_direction.xyz);
    let lambert = max(dot(normal, to_sun), 0.0);

    // Hemisphere ambient: a face turned up sees sky, one turned down sees
    // bounce off the ground. A single constant here is what makes every
    // shaded surface in a scene the same dead colour.
    let sky_amount = normal.y * 0.5 + 0.5;
    let ambient = mix(frame.ground_color.rgb, frame.sky_color.rgb, sky_amount);

    var color = in.base_color * (ambient + frame.sun_color.rgb * lambert);

    let distance = length(in.world_position - frame.camera_position.xyz);
    let span = max(frame.fog_range.y - frame.fog_range.x, 0.001);
    let fog = clamp((distance - frame.fog_range.x) / span, 0.0, 1.0);
    color = mix(color, frame.fog_color.rgb, fog);

    // An unlit surface takes neither the light nor the fog: it is not a
    // surface the sun falls on, it is something that emits. Selecting with a
    // mix rather than branching keeps both paths on the same instruction
    // stream, which matters because the two are interleaved in one draw.
    return vec4<f32>(mix(color, in.base_color, in.unlit), 1.0);
}
