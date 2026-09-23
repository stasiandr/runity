// Temporal antialiasing (taa.rs): this frame, blended into where each pixel
// was last frame, the history clipped to the colours round it now.

struct Taa {
    inverse_view_projection: mat4x4<f32>,
    previous_view_projection: mat4x4<f32>,
    // width, height, 1/width, 1/height
    size: vec4<f32>,
    // 1 when there is a history; how much of this frame to take
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> taa: Taa;
@group(0) @binding(1) var current: texture_2d<f32>;
@group(0) @binding(2) var history: texture_2d<f32>;
@group(0) @binding(3) var depth: texture_depth_2d;
@group(0) @binding(4) var linear_sampler: sampler;

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

fn to_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.25 * c.r + 0.5 * c.g + 0.25 * c.b,
        0.5 * c.r - 0.5 * c.b,
        -0.25 * c.r + 0.5 * c.g - 0.25 * c.b,
    );
}

fn from_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

// Bright pixels count for less: HDR sunlight averaged straight would win
// every blend and smear. Karis's weighting.
fn tamed(c: vec3<f32>) -> vec3<f32> {
    return c / (1.0 + max(c.r, max(c.g, c.b)));
}

fn untamed(c: vec3<f32>) -> vec3<f32> {
    return c / max(1.0 - max(c.r, max(c.g, c.b)), 1e-4);
}

// The history at `uv`, sharpened: a Catmull-Rom filter from five bilinear
// taps, so what is blended in over and over does not go soft.
fn history_at(uv: vec2<f32>) -> vec3<f32> {
    let position = uv * taa.size.xy;
    let centre = floor(position - 0.5) + 0.5;
    let f = position - centre;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let texel = taa.size.zw;
    let t0 = (centre - 1.0) * texel;
    let t3 = (centre + 2.0) * texel;
    let t12 = (centre + w2 / w12) * texel;
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    let a = w12.x * w0.y;
    sum += textureSampleLevel(history, linear_sampler, vec2<f32>(t12.x, t0.y), 0.0).rgb * a;
    let b = w0.x * w12.y;
    sum += textureSampleLevel(history, linear_sampler, vec2<f32>(t0.x, t12.y), 0.0).rgb * b;
    let c = w12.x * w12.y;
    sum += textureSampleLevel(history, linear_sampler, vec2<f32>(t12.x, t12.y), 0.0).rgb * c;
    let d = w3.x * w12.y;
    sum += textureSampleLevel(history, linear_sampler, vec2<f32>(t3.x, t12.y), 0.0).rgb * d;
    let e = w12.x * w3.y;
    sum += textureSampleLevel(history, linear_sampler, vec2<f32>(t12.x, t3.y), 0.0).rgb * e;
    weight = a + b + c + d + e;
    return max(sum / max(weight, 1e-4), vec3<f32>(0.0));
}

// Pull `h` along the line to the neighbourhood's middle until it is inside
// the box: a clip, not a clamp, so the colour does not shift hue.
fn clip_to(h: vec3<f32>, low: vec3<f32>, high: vec3<f32>) -> vec3<f32> {
    let middle = 0.5 * (high + low);
    let half = 0.5 * (high - low) + 1e-4;
    let off = h - middle;
    let scale = abs(off / half);
    let most = max(scale.x, max(scale.y, scale.z));
    if most > 1.0 {
        return middle + off / most;
    }
    return h;
}

@fragment
fn fs_resolve(in: Varyings) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let limit = vec2<i32>(taa.size.xy) - vec2<i32>(1);
    let here = textureLoad(current, pixel, 0).rgb;
    if taa.params.x < 0.5 {
        return vec4<f32>(here, 1.0);
    }
    // The neighbourhood: its mean and spread, and the nearest depth in it —
    // an edge moves with what is in front.
    var m1 = vec3<f32>(0.0);
    var m2 = vec3<f32>(0.0);
    var nearest = 1.0;
    var nearest_at = pixel;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let at = clamp(pixel + vec2<i32>(x, y), vec2<i32>(0), limit);
            let c = to_ycocg(tamed(textureLoad(current, at, 0).rgb));
            m1 += c;
            m2 += c * c;
            let d = textureLoad(depth, at, 0);
            if d < nearest {
                nearest = d;
                nearest_at = at;
            }
        }
    }
    let mean = m1 / 9.0;
    let sigma = sqrt(max(m2 / 9.0 - mean * mean, vec3<f32>(0.0)));
    let low = mean - sigma * 1.25;
    let high = mean + sigma * 1.25;

    // Where this pixel was last frame.
    let uv = (vec2<f32>(pixel) + 0.5) * taa.size.zw;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let world = taa.inverse_view_projection * vec4<f32>(ndc, nearest, 1.0);
    let was = taa.previous_view_projection * vec4<f32>(world.xyz / world.w, 1.0);
    let then = was.xy / was.w;
    let back = vec2<f32>(then.x * 0.5 + 0.5, 0.5 - then.y * 0.5);
    if was.w <= 0.0 || any(back < vec2<f32>(0.0)) || any(back > vec2<f32>(1.0)) {
        return vec4<f32>(here, 1.0);
    }
    let before = to_ycocg(tamed(history_at(back)));
    let kept = clip_to(before, low, high);
    // Moving fast, the history is blurrier: take more of this frame.
    let moved = length((back - uv) * taa.size.xy);
    let take = mix(taa.params.y, 0.4, clamp(moved / 12.0, 0.0, 1.0));
    let blended = mix(kept, to_ycocg(tamed(here)), take);
    return vec4<f32>(untamed(from_ycocg(blended)), 1.0);
}
