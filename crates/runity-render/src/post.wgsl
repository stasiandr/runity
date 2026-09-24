// Post-processing: what URP's Volume does to a frame after it is drawn.
//
// The scene is drawn into a high-dynamic-range buffer, in linear light and
// with no ceiling: the sun off a white wall is brighter than 1.0, and an
// ember is brighter still. Everything here turns that into a picture —
// bloom from what is brighter than the threshold, then exposure, white
// balance and grading, then the tonemapper that brings it into range, and
// last what a lens adds: vignette, chromatic fringes and grain.

struct Post {
    // exposure multiplier, bloom intensity, tonemapper (0 none, 1 neutral,
    // 2 ACES), encode to sRGB by hand (1) or leave it to the target (0)
    a: vec4<f32>,
    // bloom tint rgb, film grain
    bloom_tint: vec4<f32>,
    // colour filter rgb, contrast multiplier
    filter_contrast: vec4<f32>,
    // saturation multiplier, hue shift (turns), chromatic aberration, time
    b: vec4<f32>,
    // white balance as LMS scales, xyz; w is 1 to dither
    white_balance: vec4<f32>,
    // vignette colour rgb, intensity
    vignette_color: vec4<f32>,
    // vignette centre xy, smoothness, aspect
    vignette: vec4<f32>,
    // one texel of the source, and of the target
    texel: vec4<f32>,
    // bloom: threshold, knee, scatter, clamp
    bloom: vec4<f32>,
    // Channel Mixer: each output channel's shares of r, g, b
    mixer_red: vec4<f32>,
    mixer_green: vec4<f32>,
    mixer_blue: vec4<f32>,
    // Shadows Midtones Highlights: colours; w the ranges (shadows start,
    // shadows end, highlights start; lift.w highlights end)
    smh_shadows: vec4<f32>,
    smh_midtones: vec4<f32>,
    smh_highlights: vec4<f32>,
    // Lift Gamma Gain
    lift: vec4<f32>,
    gamma: vec4<f32>,
    gain: vec4<f32>,
    // Split Toning: shadows tint (w: balance), highlights tint
    split_shadows: vec4<f32>,
    split_highlights: vec4<f32>,
    // Lens Distortion: centre (-1..1), x and y amounts; theta (or its
    // inverse), sigma, 1/scale, intensity (x100)
    distortion_axis: vec4<f32>,
    distortion: vec4<f32>,
    // Panini: the view's half extents, distance, crop zoom
    panini: vec4<f32>,
    // Lens flare: tint, intensity; ghosts, halo, streaks, dispersion
    flare_tint: vec4<f32>,
    flare: vec4<f32>,
    // Lamps' own flares: count; each one's uv, intensity; its colour
    lamp_count: vec4<f32>,
    lamps: array<vec4<f32>, 8>,
    lamp_colors: array<vec4<f32>, 8>,
    // how much it is night: the eye sees grey and blue
    night: vec4<f32>,
};

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var linear_clamp: sampler;
@group(0) @binding(3) var bloom_texture: texture_2d<f32>;
// The exposure the eye has got used to, in stops (exposure.rs).
@group(0) @binding(4) var<storage, read> adapted: array<f32, 4>;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One triangle that covers the screen: cheaper than two, and no seam down
// the diagonal where two triangles' derivatives disagree.
@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> Varyings {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var out: Varyings;
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Thirteen taps in the pattern of Jimenez's "Next Generation Post
// Processing in Call of Duty": four overlapping 2x2 boxes and a centre one,
// weighted so a single bright pixel does not flicker as it crosses texels.
fn downsample13(uv: vec2<f32>, texel: vec2<f32>) -> vec3<f32> {
    let a = textureSampleLevel(source, linear_clamp, uv + texel * vec2<f32>(-2.0, -2.0), 0.0).rgb;
    let b = textureSampleLevel(source, linear_clamp, uv + texel * vec2<f32>(0.0, -2.0), 0.0).rgb;
    let c = textureSampleLevel(source, linear_clamp, uv + texel * vec2<f32>(2.0, -2.0), 0.0).rgb;
    let d = textureSampleLevel(source, linear_clamp, uv + texel * vec2<f32>(-2.0, 0.0), 0.0).rgb;
    let e = textureSampleLevel(source, linear_clamp, uv, 0.0).rgb;
    let f = textureSampleLevel(source, linear_clamp, uv + texel * vec2<f32>(2.0, 0.0), 0.0).rgb;
    let g = textureSampleLevel(source, linear_clamp, uv + texel * vec2<f32>(-2.0, 2.0), 0.0).rgb;
    let h = textureSampleLevel(source, linear_clamp, uv + texel * vec2<f32>(0.0, 2.0), 0.0).rgb;
    let i = textureSampleLevel(source, linear_clamp, uv + texel * vec2<f32>(2.0, 2.0), 0.0).rgb;
    let j = textureSampleLevel(source, linear_clamp, uv + texel * vec2<f32>(-1.0, -1.0), 0.0).rgb;
    let k = textureSampleLevel(source, linear_clamp, uv + texel * vec2<f32>(1.0, -1.0), 0.0).rgb;
    let l = textureSampleLevel(source, linear_clamp, uv + texel * vec2<f32>(-1.0, 1.0), 0.0).rgb;
    let m = textureSampleLevel(source, linear_clamp, uv + texel * vec2<f32>(1.0, 1.0), 0.0).rgb;
    return e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
}

// The first step down: only what is brighter than the threshold, with a
// soft knee so the edge of what glows is not a hard line.
@fragment
fn fs_prefilter(in: Varyings) -> @location(0) vec4<f32> {
    let c = min(downsample13(in.uv, post.texel.xy) * exp2(adapted[0]), vec3<f32>(post.bloom.w));
    let brightness = max(c.r, max(c.g, c.b));
    let knee = max(post.bloom.y, 1e-4);
    var soft = clamp(brightness - post.bloom.x + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee);
    let contribution = max(soft, brightness - post.bloom.x) / max(brightness, 1e-4);
    return vec4<f32>(c * contribution, 1.0);
}

@fragment
fn fs_downsample(in: Varyings) -> @location(0) vec4<f32> {
    return vec4<f32>(downsample13(in.uv, post.texel.xy), 1.0);
}

// Up one step: a 3x3 tent over the smaller level, added onto the larger
// one by the blend state, scaled by how far the glow is to spread.
@fragment
fn fs_upsample(in: Varyings) -> @location(0) vec4<f32> {
    let t = post.texel.xy;
    var sum = textureSampleLevel(source, linear_clamp, in.uv, 0.0).rgb * 4.0;
    sum += textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(-t.x, 0.0), 0.0).rgb * 2.0;
    sum += textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(t.x, 0.0), 0.0).rgb * 2.0;
    sum += textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(0.0, -t.y), 0.0).rgb * 2.0;
    sum += textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(0.0, t.y), 0.0).rgb * 2.0;
    sum += textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(-t.x, -t.y), 0.0).rgb;
    sum += textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(t.x, -t.y), 0.0).rgb;
    sum += textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(-t.x, t.y), 0.0).rgb;
    sum += textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(t.x, t.y), 0.0).rgb;
    return vec4<f32>(sum / 16.0 * post.bloom.z, 1.0);
}

// Unity's neutral tonemapper (John Hable's curve with URP's constants):
// compresses highlights and leaves hue and mid-tones nearly alone.
fn neutral_curve(x: vec3<f32>) -> vec3<f32> {
    let a = 0.2;
    let b = 0.29;
    let c = 0.24;
    let d = 0.272;
    let e = 0.02;
    let f = 0.3;
    return ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f)) - e / f;
}

fn tonemap_neutral(x: vec3<f32>) -> vec3<f32> {
    let white = 1.0 / neutral_curve(vec3<f32>(5.3)).x;
    return clamp(neutral_curve(x * white) * white, vec3<f32>(0.0), vec3<f32>(1.0));
}

// ACES, Stephen Hill's fit of the reference and output transforms: the
// filmic look, highlights desaturating on their way to white.
fn tonemap_aces(color: vec3<f32>) -> vec3<f32> {
    let input = mat3x3<f32>(
        vec3<f32>(0.59719, 0.07600, 0.02840),
        vec3<f32>(0.35458, 0.90834, 0.13383),
        vec3<f32>(0.04823, 0.01566, 0.83777),
    );
    let output = mat3x3<f32>(
        vec3<f32>(1.60475, -0.10208, -0.00327),
        vec3<f32>(-0.53108, 1.10813, -0.07276),
        vec3<f32>(-0.07367, -0.00605, 1.07602),
    );
    let v = input * color;
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return clamp(output * (a / b), vec3<f32>(0.0), vec3<f32>(1.0));
}

// AgX, the minimal fit (Benjamin Wrensch's of Troy Sobotka's): into the
// AgX space, a log encoding from -12.47 to +4.03 stops, the default
// contrast as a polynomial, and back out to linear for the display.
fn agx_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

fn tonemap_agx(color: vec3<f32>) -> vec3<f32> {
    let inset = mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
    );
    let outset = mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
    );
    let low = -12.47393;
    let high = 4.026069;
    var v = inset * max(color, vec3<f32>(1e-10));
    v = clamp(log2(v), vec3<f32>(low), vec3<f32>(high));
    v = (v - low) / (high - low);
    v = agx_contrast(v);
    v = outset * v;
    // The curve's output is display-encoded; back to linear, for the
    // target's own encoding.
    return clamp(pow(max(v, vec3<f32>(0.0)), vec3<f32>(2.2)), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn rgb_to_hsv(c: vec3<f32>) -> vec3<f32> {
    let k = vec4<f32>(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let p = mix(vec4<f32>(c.bg, k.wz), vec4<f32>(c.gb, k.xy), step(c.b, c.g));
    let q = mix(vec4<f32>(p.xyw, c.r), vec4<f32>(c.r, p.yzx), step(p.x, c.r));
    let d = q.x - min(q.w, q.y);
    let e = 1e-4;
    return vec3<f32>(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

fn hsv_to_rgb(c: vec3<f32>) -> vec3<f32> {
    let k = vec4<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(c.xxx + k.xyz) * 6.0 - k.www);
    return c.z * mix(k.xxx, clamp(p - k.xxx, vec3<f32>(0.0), vec3<f32>(1.0)), c.y);
}

fn white_balance(c: vec3<f32>) -> vec3<f32> {
    let to_lms = mat3x3<f32>(
        vec3<f32>(3.90405e-1, 7.08416e-2, 2.31082e-2),
        vec3<f32>(5.49941e-1, 9.63172e-1, 1.28021e-1),
        vec3<f32>(8.92632e-3, 1.35775e-3, 9.36245e-1),
    );
    let from_lms = mat3x3<f32>(
        vec3<f32>(2.85847e+0, -2.10182e-1, -4.18120e-2),
        vec3<f32>(-1.62879e+0, 1.15820e+0, -1.18169e-1),
        vec3<f32>(-2.48910e-2, 3.24281e-4, 1.06867e+0),
    );
    return from_lms * ((to_lms * c) * post.white_balance.xyz);
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

/// Half a step of an 8-bit screen either way, triangular: gradients come out
/// smooth rather than banded, and nothing visible is added.
fn dither(color: vec3<f32>, pixel: vec2<f32>) -> vec3<f32> {
    if post.white_balance.w < 0.5 {
        return color;
    }
    let noise = hash(pixel) + hash(pixel + vec2<f32>(17.0, 59.0)) - 1.0;
    return color + noise / 255.0;
}

fn hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(443.897, 441.423));
    let r = q + dot(q, q.yx + 19.19);
    return fract((r.x + r.y) * r.x);
}

// Everything after bloom, in the order URP's uber pass does it.
// Panini: where on the flat picture a point of the cylinder's lies —
// URP's Panini_Generic, by the tangent-secant theorem.
fn panini_uv(uv: vec2<f32>) -> vec2<f32> {
    let d = post.panini.z;
    if d <= 0.0 {
        return uv;
    }
    let view_pos = (2.0 * uv - 1.0) * post.panini.xy * post.panini.w;
    let view_dist = 1.0 + d;
    let view_hyp_sq = view_pos.x * view_pos.x + view_dist * view_dist;
    let isect_d = view_pos.x * d;
    let isect_discrim = view_hyp_sq - isect_d * isect_d;
    let cyl_dist_minus_d = (-isect_d * view_pos.x + view_dist * sqrt(isect_discrim)) / view_hyp_sq;
    let cyl_dist = cyl_dist_minus_d + d;
    let cyl_pos = view_pos * (cyl_dist / view_dist);
    let proj = cyl_pos / (cyl_dist - d);
    return proj / post.panini.xy * 0.5 + 0.5;
}

// Lens Distortion, URP's DistortUV: barrel by a tangent, pincushion by
// an arctangent, about the centre.
fn distort_uv(uv_in: vec2<f32>) -> vec2<f32> {
    let intensity = post.distortion.w;
    if abs(intensity) < 1e-3 {
        return uv_in;
    }
    var uv = (uv_in - 0.5) * post.distortion.z + 0.5;
    let ruv = post.distortion_axis.zw * (uv - 0.5 - post.distortion_axis.xy);
    var ru = max(length(ruv), 1e-5);
    if intensity > 0.0 {
        let wu = ru * post.distortion.x;
        ru = tan(wu) / (ru * post.distortion.y);
    } else {
        ru = (1.0 / ru) * post.distortion.x * atan(ru * post.distortion.y);
    }
    return uv + ruv * (ru - 1.0);
}

// The bloom where it is far past white: a flare is made of the few
// brightest things, not of every lit wall.
fn flare_source(at: vec2<f32>) -> vec3<f32> {
    let c = textureSampleLevel(bloom_texture, linear_clamp, at, 0.0).rgb;
    let over = max(luma(c) - 2.0, 0.0);
    let share = over / max(luma(c), 1e-4);
    return c * share * share;
}

// Screen-space lens flare from the bloom: ghosts of the bright spots
// mirrored through the middle, a halo ring, and a horizontal streak
// (Chapman's pseudo lens flare, with URP's knobs).
fn lens_flare(uv: vec2<f32>) -> vec3<f32> {
    if post.flare_tint.w <= 0.0 {
        return vec3<f32>(0.0);
    }
    let flipped = vec2<f32>(1.0) - uv;
    let ghost_step = (vec2<f32>(0.5) - flipped) * 0.37;
    let spread = normalize(ghost_step + vec2<f32>(1e-5)) * post.flare.w * 0.01;
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < 5; i = i + 1) {
        let at = fract(flipped + ghost_step * f32(i));
        let weight = pow(1.0 - clamp(length(vec2<f32>(0.5) - at) / 0.7071, 0.0, 1.0), 10.0);
        sum += vec3<f32>(
            flare_source(at + spread).r,
            flare_source(at).g,
            flare_source(at - spread).b,
        ) * weight;
    }
    var color = sum * post.flare.x;
    // The halo: a ring a fixed way out from the middle.
    // Round whatever the screen's shape: the direction is taken in square
    // units and put back.
    let aspect = vec2<f32>(post.vignette.w, 1.0);
    let halo_dir = normalize((ghost_step + vec2<f32>(1e-5)) * aspect) / aspect;
    let halo_at = fract(flipped + halo_dir * 0.3);
    let halo_weight = pow(1.0 - clamp(length(vec2<f32>(0.5) - halo_at) / 0.7071, 0.0, 1.0), 5.0);
    color += flare_source(halo_at) * halo_weight * post.flare.y;
    // The streak: the bloom smeared sideways.
    var streak = vec3<f32>(0.0);
    for (var i = -6; i <= 6; i = i + 1) {
        let o = f32(i) / 6.0;
        streak += flare_source(uv + vec2<f32>(o * 0.25, 0.0)) * (1.0 - abs(o));
    }
    color += streak / 7.0 * post.flare.z;
    return color * post.flare_tint.rgb * post.flare_tint.w * 0.25;
}

// A soft round spot on the picture, round whatever its shape.
fn spot(uv: vec2<f32>, at: vec2<f32>, radius: f32) -> f32 {
    let d = (uv - at) * vec2<f32>(post.vignette.w, 1.0);
    let x = clamp(1.0 - length(d) / radius, 0.0, 1.0);
    return x * x;
}

// Each lamp's own flare (Unity's Lens Flare SRP component): a glow round
// it, a thin ring, and ghosts along the line through the middle of the
// picture. Seen only as much as the lamp is: the bloom where it stands is
// bright when it is in sight and dark behind a wall.
fn lamp_flares(uv: vec2<f32>) -> vec3<f32> {
    var color = vec3<f32>(0.0);
    let count = i32(post.lamp_count.x);
    for (var i = 0; i < count; i = i + 1) {
        let lamp = post.lamps[i];
        let at = lamp.xy;
        let seen_at = clamp(at, vec2<f32>(0.0), vec2<f32>(1.0));
        let seen = clamp(luma(textureSampleLevel(bloom_texture, linear_clamp, seen_at, 0.0).rgb) * 0.5, 0.0, 1.0);
        // Fading as it leaves the picture.
        let edge = clamp(min(min(at.x, at.y), min(1.0 - at.x, 1.0 - at.y)) * 5.0 + 1.0, 0.0, 1.0);
        let strength = lamp.z * seen * edge;
        if strength <= 0.0 {
            continue;
        }
        let tint = post.lamp_colors[i].rgb;
        var f = spot(uv, at, 0.12) * 0.6 + spot(uv, at, 0.03) * 1.5;
        let ring = (uv - at) * vec2<f32>(post.vignette.w, 1.0);
        f += 0.06 * clamp(1.0 - abs(length(ring) - 0.2) / 0.01, 0.0, 1.0);
        let axis = vec2<f32>(0.5) - at;
        f += spot(uv, at + axis * 0.6, 0.03) * 0.5;
        f += spot(uv, at + axis * 1.2, 0.06) * 0.3;
        f += spot(uv, at + axis * 1.6, 0.02) * 0.6;
        f += spot(uv, at + axis * 2.1, 0.09) * 0.2;
        color += tint * f * strength;
    }
    return color;
}

@fragment
fn fs_composite(in: Varyings) -> @location(0) vec4<f32> {
    let uv = distort_uv(panini_uv(in.uv));
    // Chromatic aberration: red and blue sampled a little apart along the
    // line from the centre, as a cheap lens fails to focus them together.
    let from_centre = uv - vec2<f32>(0.5);
    let fringe = from_centre * dot(from_centre, from_centre) * post.b.z * 0.1;
    var color = vec3<f32>(
        textureSampleLevel(source, linear_clamp, uv - fringe, 0.0).r,
        textureSampleLevel(source, linear_clamp, uv, 0.0).g,
        textureSampleLevel(source, linear_clamp, uv + fringe, 0.0).b,
    ) * exp2(adapted[0]);
    color += textureSampleLevel(bloom_texture, linear_clamp, uv, 0.0).rgb * post.a.y * post.bloom_tint.rgb;
    color += lens_flare(uv);
    color += lamp_flares(uv);

    color *= post.a.x;
    // Night: the eye's cones give up to its rods, which see no colour and
    // most in blue-green — moonlit sand is grey-blue, not orange. Only where
    // it is dark: in a lamp's pool the cones still see, and a flame stays
    // warm.
    if post.night.x > 0.0 {
        let seen = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
        let dim = 1.0 - smoothstep(0.04, 0.35, seen);
        color = mix(color, seen * vec3<f32>(0.62, 0.8, 1.12), post.night.x * 0.7 * dim);
    }
    color = white_balance(color);
    color *= post.filter_contrast.rgb;
    // Contrast about middle grey, in log space where it is even-handed.
    let log_color = log2(max(color, vec3<f32>(1e-6)) / 0.18);
    color = 0.18 * exp2(log_color * post.filter_contrast.w);
    // Channel Mixer.
    color = vec3<f32>(
        dot(color, post.mixer_red.rgb),
        dot(color, post.mixer_green.rgb),
        dot(color, post.mixer_blue.rgb),
    );
    color = max(color, vec3<f32>(0.0));
    // Shadows Midtones Highlights: by luminance, each its colour.
    let y = luma(color);
    let shadows = 1.0 - smoothstep(post.smh_shadows.w, post.smh_midtones.w, y);
    let highlights = smoothstep(post.smh_highlights.w, post.lift.w, y);
    let midtones = 1.0 - shadows - highlights;
    color = color * (post.smh_shadows.rgb * shadows + post.smh_midtones.rgb * midtones
        + post.smh_highlights.rgb * highlights);
    // Lift Gamma Gain.
    color = post.gain.rgb * (color + post.lift.rgb * (vec3<f32>(1.0) - min(color, vec3<f32>(1.0))));
    color = pow(max(color, vec3<f32>(0.0)), vec3<f32>(1.0) / post.gamma.rgb);
    // Split Toning, soft-lit in gamma space as URP does.
    if any(post.split_shadows.rgb != vec3<f32>(0.5)) || any(post.split_highlights.rgb != vec3<f32>(0.5)) {
        var g = linear_to_srgb(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)));
        let t = clamp(luma(g) + post.split_shadows.w, 0.0, 1.0);
        g = soft_light(g, mix(vec3<f32>(0.5), post.split_shadows.rgb, 1.0 - t));
        g = soft_light(g, mix(vec3<f32>(0.5), post.split_highlights.rgb, t));
        color = srgb_to_linear(g) + max(color - vec3<f32>(1.0), vec3<f32>(0.0));
    }
    if abs(post.b.y) > 1e-5 {
        var hsv = rgb_to_hsv(color);
        hsv.x = fract(hsv.x + post.b.y);
        color = hsv_to_rgb(hsv);
    }
    color = max(mix(vec3<f32>(luma(color)), color, post.b.x), vec3<f32>(0.0));

    let mode = u32(post.a.z + 0.5);
    if mode == 1u {
        color = tonemap_neutral(color);
    } else if mode == 2u {
        color = tonemap_aces(color);
    } else if mode == 3u {
        color = tonemap_agx(color);
    } else {
        color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    }

    // Vignette: darkened towards the corners, round whatever the aspect.
    let d = (in.uv - post.vignette.xy) * vec2<f32>(post.vignette.w, 1.0);
    let falloff = smoothstep(0.0, 1.0, dot(d, d) * post.vignette_color.w * 2.0);
    let edge = pow(falloff, max(1.0 - post.vignette.z, 0.05));
    color = mix(color, post.vignette_color.rgb, edge * step(1e-4, post.vignette_color.w));

    // Grain, stronger in the darks, as film is.
    let grain = hash(in.position.xy + post.b.w * 61.0) - 0.5;
    color += grain * post.bloom_tint.w * 0.25 * (1.0 - sqrt(luma(color)));
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));

    return vec4<f32>(finish(color, in.position.xy), 1.0);
}

/// Into what the screen stores: encoded here when the target does not
/// encode itself, and dithered in the encoding either way — that is where
/// a step of the screen is a step.
fn finish(color: vec3<f32>, pixel: vec2<f32>) -> vec3<f32> {
    let encoded = dither(linear_to_srgb(color), pixel);
    if post.a.w > 0.5 {
        return encoded;
    }
    return srgb_to_linear(max(encoded, vec3<f32>(0.0)));
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

// FXAA, the console version's idea in a few lines: find the edge by luma
// contrast, and blend across it along its direction. Runs on the
// tonemapped picture, in perceptual luma.
@fragment
fn fs_fxaa(in: Varyings) -> @location(0) vec4<f32> {
    let t = post.texel.xy;
    let c = textureSampleLevel(source, linear_clamp, in.uv, 0.0).rgb;
    let l = sqrt(luma(c));
    let ln = sqrt(luma(textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(0.0, -t.y), 0.0).rgb));
    let ls = sqrt(luma(textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(0.0, t.y), 0.0).rgb));
    let le = sqrt(luma(textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(t.x, 0.0), 0.0).rgb));
    let lw = sqrt(luma(textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(-t.x, 0.0), 0.0).rgb));
    let lo = min(l, min(min(ln, ls), min(le, lw)));
    let hi = max(l, max(max(ln, ls), max(le, lw)));
    var out = c;
    if hi - lo >= max(0.0312, hi * 0.125) {
        let lnw = sqrt(luma(textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(-t.x, -t.y), 0.0).rgb));
        let lne = sqrt(luma(textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(t.x, -t.y), 0.0).rgb));
        let lsw = sqrt(luma(textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(-t.x, t.y), 0.0).rgb));
        let lse = sqrt(luma(textureSampleLevel(source, linear_clamp, in.uv + vec2<f32>(t.x, t.y), 0.0).rgb));
        var dir = vec2<f32>(-((lnw + lne) - (lsw + lse)), (lnw + lsw) - (lne + lse));
        let reduce = max((lnw + lne + lsw + lse) * 0.03125, 1.0 / 128.0);
        let scale = 1.0 / (min(abs(dir.x), abs(dir.y)) + reduce);
        dir = clamp(dir * scale, vec2<f32>(-8.0), vec2<f32>(8.0)) * t;
        let a = 0.5 * (textureSampleLevel(source, linear_clamp, in.uv + dir * (1.0 / 3.0 - 0.5), 0.0).rgb
            + textureSampleLevel(source, linear_clamp, in.uv + dir * (2.0 / 3.0 - 0.5), 0.0).rgb);
        let b = a * 0.5 + 0.25 * (textureSampleLevel(source, linear_clamp, in.uv + dir * -0.5, 0.0).rgb
            + textureSampleLevel(source, linear_clamp, in.uv + dir * 0.5, 0.0).rgb);
        let lb = sqrt(luma(b));
        out = select(b, a, lb < lo || lb > hi);
    }
    return vec4<f32>(finish(out, in.position.xy), 1.0);
}

/// Photoshop's soft light: darker below mid grey, lighter above.
fn soft_light(base: vec3<f32>, blend: vec3<f32>) -> vec3<f32> {
    let low = 2.0 * base * blend + base * base * (1.0 - 2.0 * blend);
    let high = sqrt(base) * (2.0 * blend - 1.0) + 2.0 * base * (1.0 - blend);
    return select(high, low, blend < vec3<f32>(0.5));
}
