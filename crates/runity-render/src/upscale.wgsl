// Upscaling (upscale.rs): where each pixel was last frame, for a temporal
// upscaler; and the engine's own spatial one, for a device without MetalFX.

struct Upscale {
    // This frame's camera as drawn (moved by the jitter): screen and depth
    // into the world.
    inverse_view_projection: mat4x4<f32>,
    // This frame's camera, unmoved, and last frame's.
    view_projection: mat4x4<f32>,
    previous_view_projection: mat4x4<f32>,
    // The picture drawn: width, height, 1/width, 1/height.
    input: vec4<f32>,
    // The picture made: the same.
    output: vec4<f32>,
    // How much to sharpen.
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> up: Upscale;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var depth: texture_depth_2d;
@group(0) @binding(3) var linear_sampler: sampler;

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

fn to_uv(clip: vec4<f32>) -> vec2<f32> {
    let ndc = clip.xy / clip.w;
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

/// How far each pixel has come since last frame, as a share of the
/// screen, pointing back to where it was: the camera's motion alone, as
/// TAA reprojects (there are no motion vectors per object).
@fragment
fn fs_motion(in: Varyings) -> @location(0) vec2<f32> {
    let d = textureLoad(depth, vec2<i32>(in.position.xy), 0);
    let uv = in.position.xy * up.input.zw;
    let seen = up.inverse_view_projection * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let world = vec4<f32>(seen.xyz / seen.w, 1.0);
    return to_uv(up.previous_view_projection * world) - to_uv(up.view_projection * world);
}

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(picture, linear_sampler, uv, 0.0).rgb;
}

/// The picture at `uv`, by Catmull-Rom in five bilinear taps: sharper than
/// bilinear, and held inside the four texels round it so a bright sun does
/// not ring.
@fragment
fn fs_upscale(in: Varyings) -> @location(0) vec4<f32> {
    let uv = in.position.xy * up.output.zw;
    let texel = up.input.zw;
    let at = uv * up.input.xy;
    let centre = floor(at - 0.5) + 0.5;
    let f = at - centre;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let p0 = (centre - 1.0) * texel;
    let p3 = (centre + 2.0) * texel;
    let p12 = (centre + w2 / w12) * texel;
    var c = tap(vec2<f32>(p12.x, p0.y)) * (w12.x * w0.y)
        + tap(vec2<f32>(p0.x, p12.y)) * (w0.x * w12.y)
        + tap(p12) * (w12.x * w12.y)
        + tap(vec2<f32>(p3.x, p12.y)) * (w3.x * w12.y)
        + tap(vec2<f32>(p12.x, p3.y)) * (w12.x * w3.y);
    let weight = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    c = c / max(weight, 1e-4);
    // The four texels it lies between: what it may not leave.
    let base = vec2<i32>(floor(at - 0.5));
    let limit = vec2<i32>(up.input.xy) - vec2<i32>(1);
    let a = textureLoad(picture, clamp(base, vec2<i32>(0), limit), 0).rgb;
    let b = textureLoad(picture, clamp(base + vec2<i32>(1, 0), vec2<i32>(0), limit), 0).rgb;
    let e = textureLoad(picture, clamp(base + vec2<i32>(0, 1), vec2<i32>(0), limit), 0).rgb;
    let g = textureLoad(picture, clamp(base + vec2<i32>(1, 1), vec2<i32>(0), limit), 0).rgb;
    let low = min(min(a, b), min(e, g));
    let high = max(max(a, b), max(e, g));
    // A little sharper still: away from the bilinear blur, within bounds.
    let blurred = tap(uv);
    c = clamp(c + (c - blurred) * up.params.x, low, high);
    return vec4<f32>(max(c, vec3<f32>(0.0)), 1.0);
}
