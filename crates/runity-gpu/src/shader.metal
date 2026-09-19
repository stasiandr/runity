// The Metal half of every shader the engine ships.
//
// Each pair here is the twin of a `Shader` implementation in Rust, and the
// differential tests in `tests/` are what keeps the two honest: the same
// closure is drawn by the rasterizer and by this file, and the frames are
// compared pixel by pixel. If you change one side, change the other.
//
// Conventions that have to hold, and that the tests check:
//
//   * `Vertex` below is byte-for-byte `runity_render::shader::Vertex`:
//     48 bytes, no padding, which is why every member is a `packed_` type.
//     There is no `MTLVertexDescriptor`; `[[vertex_id]]` indexes the buffer.
//   * Uniform blocks are matrices first, then `float4`s, padded by hand, so
//     `sizeof` is always a multiple of 16 on both sides.
//   * Colours are linear, the target is `BGRA8Unorm` (not `_sRGB`), and no
//     stage applies a transfer curve. The CPU rasterizer does not either.

#include <metal_stdlib>
using namespace metal;

/// Byte-for-byte `runity_render::shader::Vertex`. 12 + 12 + 8 + 16 = 48.
struct Vertex {
    packed_float3 position;
    packed_float3 normal;
    packed_float2 uv;
    packed_float4 color;
};

// ---------------------------------------------------------------------------
// BasicShader — Lambert diffuse plus Blinn-Phong specular over an optional
// texture. The twin of `runity_render::shader::BasicShader`.
// ---------------------------------------------------------------------------

struct BasicUniforms {
    float4x4 model;
    float4x4 view_projection;
    float4x4 normal_matrix;
    float4 base_color;
    float4 light;       // xyz: direction the light travels, w: intensity
    float4 light_color;  // rgb: light colour, w: specular strength
    float4 ambient;      // rgb: ambient term
    float4 camera;       // xyz: eye position, w: shininess
    float4 flags;        // x: 1 when a texture is bound
};

struct BasicInOut {
    float4 position [[position]];
    float3 world_position;
    float3 normal;
    float2 uv;
    float4 color;
};

vertex BasicInOut basic_vertex(uint vid [[vertex_id]],
                               device const Vertex *vertices [[buffer(0)]],
                               constant BasicUniforms &u [[buffer(1)]]) {
    const Vertex v = vertices[vid];
    const float4 world = u.model * float4(float3(v.position), 1.0);
    BasicInOut out;
    out.position = u.view_projection * world;
    out.world_position = world.xyz;
    out.normal = (u.normal_matrix * float4(float3(v.normal), 0.0)).xyz;
    out.uv = float2(v.uv);
    out.color = float4(v.color);
    return out;
}

fragment float4 basic_fragment(BasicInOut in [[stage_in]],
                               constant BasicUniforms &u [[buffer(1)]],
                               texture2d<float> albedo_map [[texture(0)]],
                               sampler albedo_sampler [[sampler(0)]]) {
    float4 albedo = u.base_color;
    if (u.flags.x > 0.5) {
        albedo = albedo_map.sample(albedo_sampler, in.uv) * u.base_color;
    }
    albedo *= in.color;

    const float3 n = normalize(in.normal);
    const float3 to_light = -normalize(u.light.xyz);
    const float diffuse = max(dot(n, to_light), 0.0) * u.light.w;

    float specular = 0.0;
    if (diffuse > 0.0 && u.light_color.w > 0.0) {
        const float3 to_eye = normalize(u.camera.xyz - in.world_position);
        const float3 half_vector = normalize(to_light + to_eye);
        specular = pow(max(dot(n, half_vector), 0.0), u.camera.w) * u.light_color.w;
    }

    return float4(albedo.rgb * (u.ambient.rgb + u.light_color.rgb * diffuse) + specular,
                  albedo.a);
}

// ---------------------------------------------------------------------------
// UnlitShader — vertex colour times an optional texture times a tint.
// ---------------------------------------------------------------------------

struct UnlitUniforms {
    float4x4 mvp;
    float4 tint;
    float4 flags;  // x: 1 when a texture is bound
};

struct UnlitInOut {
    float4 position [[position]];
    float4 color;
    float2 uv;
};

vertex UnlitInOut unlit_vertex(uint vid [[vertex_id]],
                               device const Vertex *vertices [[buffer(0)]],
                               constant UnlitUniforms &u [[buffer(1)]]) {
    const Vertex v = vertices[vid];
    UnlitInOut out;
    out.position = u.mvp * float4(float3(v.position), 1.0);
    out.color = float4(v.color);
    out.uv = float2(v.uv);
    return out;
}

fragment float4 unlit_fragment(UnlitInOut in [[stage_in]],
                               constant UnlitUniforms &u [[buffer(1)]],
                               texture2d<float> albedo_map [[texture(0)]],
                               sampler albedo_sampler [[sampler(0)]]) {
    float4 base = float4(1.0);
    if (u.flags.x > 0.5) {
        base = albedo_map.sample(albedo_sampler, in.uv);
    }
    return base * in.color * u.tint;
}

// ---------------------------------------------------------------------------
// The pulse: vertex colour scaled per fragment, alpha untouched. Both
// `PulseShader` (smallworld) and `Gradient` (hello_triangle) are this shader.
// ---------------------------------------------------------------------------

struct PulseUniforms {
    float4x4 mvp;
    float4 params;  // x: the pulse
};

struct PulseInOut {
    float4 position [[position]];
    float4 color;
};

vertex PulseInOut pulse_vertex(uint vid [[vertex_id]],
                               device const Vertex *vertices [[buffer(0)]],
                               constant PulseUniforms &u [[buffer(1)]]) {
    const Vertex v = vertices[vid];
    PulseInOut out;
    out.position = u.mvp * float4(float3(v.position), 1.0);
    out.color = float4(v.color);
    return out;
}

fragment float4 pulse_fragment(PulseInOut in [[stage_in]],
                               constant PulseUniforms &u [[buffer(1)]]) {
    return float4(in.color.rgb * u.params.x, in.color.a);
}

// ---------------------------------------------------------------------------
// Debug lines. Screen-space segments, expanded to quads on the CPU side and
// handed here as ordinary vertices with `position` already in clip space.
// ---------------------------------------------------------------------------

struct LineInOut {
    float4 position [[position]];
    float4 color;
};

vertex LineInOut line_vertex(uint vid [[vertex_id]],
                             device const Vertex *vertices [[buffer(0)]],
                             constant PulseUniforms &u [[buffer(1)]]) {
    const Vertex v = vertices[vid];
    LineInOut out;
    out.position = u.mvp * float4(float3(v.position), 1.0);
    out.color = float4(v.color);
    return out;
}

fragment float4 line_fragment(LineInOut in [[stage_in]]) {
    return in.color;
}

// ---------------------------------------------------------------------------
// The echo shaders. Neither draws anything a game wants; both exist to read
// raw GPU-side memory back out as pixels, so that a disagreement about a
// struct layout fails on the one field that is wrong instead of somewhere
// downstream in a lit frame.
// ---------------------------------------------------------------------------

/// Writes uniform word `y * 4 + x` into the pixel at `(x, y)`. Every field of
/// the block reaches its own pixel, so a mismatch names the offending word.
struct EchoInOut {
    float4 position [[position]];
    /// Vertex 0's `normal`, `uv` and `color`, flat so no interpolation can
    /// smear the values being probed. The provoking vertex of a triangle is
    /// its first index, and the probe mesh is one triangle.
    float4 probe0 [[flat]];
    float4 probe1 [[flat]];
    float4 probe2 [[flat]];
};

vertex EchoInOut echo_vertex(uint vid [[vertex_id]],
                             device const Vertex *vertices [[buffer(0)]],
                             constant float4 *u [[buffer(1)]]) {
    const Vertex v = vertices[vid];
    const Vertex probe = vertices[0];
    EchoInOut out;
    // The probe mesh carries clip-space positions directly: no matrix, so a
    // failure here is a layout failure and not an arithmetic one.
    out.position = float4(float3(v.position), 1.0);
    out.probe0 = float4(float3(probe.normal), probe.uv[0]);
    out.probe1 = float4(probe.uv[1], float4(probe.color).rgb);
    out.probe2 = float4(float4(probe.color).a, 0.0, 0.0, 0.0);
    return out;
}

fragment float4 echo_uniform_fragment(EchoInOut in [[stage_in]],
                                      constant float *u [[buffer(1)]]) {
    const uint2 p = uint2(in.position.xy);
    return float4(u[p.y * 4u + p.x], 0.0, 0.0, 1.0);
}

fragment float4 echo_vertex_fragment(EchoInOut in [[stage_in]],
                                     constant float *u [[buffer(1)]]) {
    const uint index = uint(in.position.x);
    const float words[9] = {in.probe0.x, in.probe0.y, in.probe0.z, in.probe0.w,
                            in.probe1.x, in.probe1.y, in.probe1.z, in.probe1.w,
                            in.probe2.x};
    return float4(words[min(index, 8u)], 0.0, 0.0, 1.0);
}
