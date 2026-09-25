// What a camera's lens and shutter do to the frame before it is graded:
// URP's Depth of Field and Motion Blur. Both read the scene's depth from
// the prepass (what is solid), and both work on the high-dynamic-range
// picture, so a bright highlight out of focus spreads into a bright disc.

struct Lens {
    inverse_view_projection: mat4x4<f32>,
    // Last frame's: where each point was on screen a frame ago.
    previous_view_projection: mat4x4<f32>,
    // near, far, 1 for an orthographic camera
    depth_range: vec4<f32>,
    // mode (1 Gaussian, 2 Bokeh), focus distance (m), focal length (m),
    // aperture (f-number)
    focus: vec4<f32>,
    // Gaussian start, end (m); the largest blur in pixels; pixels to the
    // sensor's metre (Bokeh)
    blur: vec4<f32>,
    // width, height, 1/width, 1/height
    size: vec4<f32>,
    // motion blur: intensity, clamp (share of the screen), samples
    motion: vec4<f32>,
    view_projection: mat4x4<f32>,
    // the eye; w time in seconds
    eye: vec4<f32>,
    // heat haze: shimmer, mirage, where they start (m)
    heat: vec4<f32>,
};

@group(0) @binding(0) var<uniform> lens: Lens;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var depth: texture_depth_2d;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> Varyings {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var out: Varyings;
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

fn world_at(uv: vec2<f32>) -> vec3<f32> {
    let limit = vec2<i32>(lens.size.xy) - vec2<i32>(1);
    let pixel = clamp(vec2<i32>(uv * lens.size.xy), vec2<i32>(0), limit);
    let d = textureLoad(depth, pixel, 0);
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let world = lens.inverse_view_projection * ndc;
    return world.xyz / world.w;
}

/// How deep a pixel is, in metres along the view: the depth buffer's
/// value put back through the projection, with no matrix.
fn distance_at(uv: vec2<f32>) -> f32 {
    let limit = vec2<i32>(lens.size.xy) - vec2<i32>(1);
    let pixel = clamp(vec2<i32>(uv * lens.size.xy), vec2<i32>(0), limit);
    let d = textureLoad(depth, pixel, 0);
    let near = lens.depth_range.x;
    let far = lens.depth_range.y;
    if lens.depth_range.z > 0.5 {
        return near + d * (far - near);
    }
    return near * far / (far - d * (far - near));
}

/// How wide, in pixels, a point this far away is blurred.
fn circle_of_confusion(d: f32) -> f32 {
    let mode = lens.focus.x;
    let largest = lens.blur.z;
    if mode < 1.5 {
        // Gaussian: nothing near, blurring in from `start` to `end`.
        return smoothstep(lens.blur.x, lens.blur.y, d) * largest;
    }
    // Bokeh: a thin lens. f²·|d − s| / (N·d·(s − f)), on the sensor, in
    // pixels.
    let s = lens.focus.y;
    let f = lens.focus.z;
    let n = lens.focus.w;
    let on_sensor = f * f * abs(d - s) / max(n * d * max(s - f, 1e-4), 1e-6);
    return min(on_sensor * lens.blur.w, largest);
}

// Depth of field by gathering: a spiral of taps around the pixel out to
// the largest blur, each counted when its own circle of confusion reaches
// this far — so a blurred thing spreads over what is behind it, and what is
// sharp behind a blurred thing stays sharp where it is not covered. A tap
// behind the pixel spreads no wider than twice the pixel's own blur, so a
// sharp object does not bleed the background's blur over itself.
// (Gustafsson's single-pass scatter-as-gather.)
/// A colour that is a number, and one half floats hold: a NaN or an
/// infinity in one pixel (a glint past the format's top) is black, not a
/// disc of it through the blur — URP's Stop NaN.
fn finite(c: vec3<f32>) -> vec3<f32> {
    let bad = (c != c) | (abs(c) > vec3<f32>(65000.0));
    return select(c, vec3<f32>(0.0), bad);
}

@fragment
fn fs_depth_of_field(in: Varyings) -> @location(0) vec4<f32> {
    let centre_depth = distance_at(in.uv);
    let centre_size = circle_of_confusion(centre_depth);
    var color = finite(textureSampleLevel(source, linear_sampler, in.uv, 0.0).rgb);
    var total = 1.0;
    let largest = lens.blur.z;
    if largest < 0.5 {
        return vec4<f32>(color, 1.0);
    }
    // The step grows the spiral so about sixty taps cover the disc.
    let scale = max(largest * largest / 120.0, 0.25);
    var radius = scale;
    var angle = 0.0;
    for (var i = 0; i < 256; i = i + 1) {
        if radius >= largest {
            break;
        }
        let tap = in.uv + vec2<f32>(cos(angle), sin(angle)) * lens.size.zw * radius;
        let tap_color = finite(textureSampleLevel(source, linear_sampler, tap, 0.0).rgb);
        let tap_depth = distance_at(tap);
        var tap_size = circle_of_confusion(tap_depth);
        if tap_depth > centre_depth {
            tap_size = clamp(tap_size, 0.0, centre_size * 2.0);
        }
        let m = smoothstep(radius - 0.5, radius + 0.5, tap_size);
        color = color + mix(color / total, tap_color, m);
        total = total + 1.0;
        radius = radius + scale / radius;
        angle = angle + 2.39996323;
    }
    return vec4<f32>(color / total, 1.0);
}

// Camera motion blur: where each point was a frame ago, from the depth and
// last frame's view, and the picture averaged along the way from there to
// here — URP's Camera mode, which blurs what the camera's own movement
// sweeps, not what moves by itself.
@fragment
fn fs_motion_blur(in: Varyings) -> @location(0) vec4<f32> {
    let world = world_at(in.uv);
    let before = lens.previous_view_projection * vec4<f32>(world, 1.0);
    var velocity = vec2<f32>(0.0);
    if before.w > 1e-4 {
        let ndc = before.xy / before.w;
        let was = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        velocity = (in.uv - was) * lens.motion.x;
    }
    // Clamped to a share of the screen, so a camera cut is not a smear.
    let most = lens.motion.y;
    let speed = length(velocity);
    if speed > most {
        velocity = velocity * (most / speed);
    }
    let count = max(u32(lens.motion.z), 2u);
    var color = vec3<f32>(0.0);
    for (var i = 0u; i < count; i = i + 1u) {
        let t = f32(i) / f32(count - 1u) - 0.5;
        color = color + textureSampleLevel(source, linear_sampler, in.uv - velocity * t, 0.0).rgb;
    }
    return vec4<f32>(color / f32(count), 1.0);
}

fn heat_hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(123.34, 456.21));
    let r = q + dot(q, q + 45.32);
    return fract(r.x * r.y);
}

fn heat_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(heat_hash(i), heat_hash(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(heat_hash(i + vec2<f32>(0.0, 1.0)), heat_hash(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

// Hot air. The picture shimmers where the view crosses a lot of it — far
// off and near the horizon — with ripples rising as heat does. Far off,
// just below the horizon, the ground gives way to the sky seen mirrored
// across it: the mirage, shimmering too.
@fragment
fn fs_heat_haze(in: Varyings) -> @location(0) vec4<f32> {
    let t = lens.eye.w;
    let far_off = distance_at(in.uv);
    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let a = lens.inverse_view_projection * vec4<f32>(ndc, 0.0, 1.0);
    let b = lens.inverse_view_projection * vec4<f32>(ndc, 1.0, 1.0);
    let direction = normalize(b.xyz * a.w - a.xyz * b.w);
    let start = lens.heat.z;

    let through = smoothstep(start, start * 4.0 + 1.0, far_off);
    let near_horizon = 1.0 - smoothstep(0.0, 0.3, abs(direction.y));
    let shimmer = lens.heat.x * through * near_horizon;
    let q = in.uv * vec2<f32>(70.0, 35.0);
    let wobble = vec2<f32>(
        heat_noise(q + vec2<f32>(0.0, t * 2.2)),
        heat_noise(q * 1.3 + vec2<f32>(3.1, t * 2.9)),
    ) - 0.5;
    let uv = in.uv + wobble * shimmer * 0.006;
    var color = textureSampleLevel(source, linear_sampler, uv, 0.0).rgb;

    let below = -direction.y;
    let ground = f32(textureLoad(depth, clamp(vec2<i32>(in.uv * lens.size.xy), vec2<i32>(0), vec2<i32>(lens.size.xy) - vec2<i32>(1)), 0) < 1.0);
    if lens.heat.y > 0.0 && below > 0.0 && ground > 0.5 {
        let mirage = lens.heat.y * smoothstep(start * 1.5, start * 4.0 + 1.0, far_off)
            * (1.0 - smoothstep(0.01, 0.09, below));
        if mirage > 0.0 {
            // The same way, turned up across the horizon.
            let up = normalize(vec3<f32>(direction.x, below, direction.z));
            let far = lens.view_projection * vec4<f32>(lens.eye.xyz + up * 1000.0, 1.0);
            let seen = far.xy / far.w;
            let mirrored = vec2<f32>(seen.x * 0.5 + 0.5, 0.5 - seen.y * 0.5) + wobble * 0.01;
            let sky = textureSampleLevel(source, linear_sampler, clamp(mirrored, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
            color = mix(color, sky, mirage);
        }
    }
    return vec4<f32>(color, 1.0);
}
