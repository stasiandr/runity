// scrap-ui: rounded rectangles and pictures.
//
// Everything is in physical pixels from the top left. Colours arrive as the
// design file writes them — sRGB, straight alpha — and the target is the
// plain-bytes view of an sRGB texture, so blending happens on sRGB values,
// as in a browser. The fragment works out the distance to the rounded box
// and fades over one pixel, which is the anti-aliasing; the border is the
// band within `border_width` of the edge.

struct Screen {
    size: vec2<f32>,
    _pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> screen: Screen;

struct Shape {
    @location(0) rect: vec4<f32>,
    @location(1) fill: vec4<f32>,
    @location(2) border: vec4<f32>,
    // radius, border width, unused, unused
    @location(3) params: vec4<f32>,
    @location(4) clip: vec4<f32>,
};

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) rect: vec4<f32>,
    @location(1) fill: vec4<f32>,
    @location(2) border: vec4<f32>,
    @location(3) params: vec4<f32>,
    @location(4) clip: vec4<f32>,
    @location(5) uv: vec2<f32>,
};

fn to_clip_space(p: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(p.x / screen.size.x * 2.0 - 1.0, 1.0 - p.y / screen.size.y * 2.0, 0.0, 1.0);
}

@vertex
fn vs(@builtin(vertex_index) index: u32, shape: Shape) -> Out {
    let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
    // One pixel of room around the box for its soft edge.
    let grow = 1.0;
    let p = shape.rect.xy - vec2<f32>(grow) + corner * (shape.rect.zw + vec2<f32>(2.0 * grow));
    var out: Out;
    out.pos = to_clip_space(p);
    out.rect = shape.rect;
    out.fill = shape.fill;
    out.border = shape.border;
    out.params = shape.params;
    out.clip = shape.clip;
    out.uv = (p - shape.rect.xy) / max(shape.rect.zw, vec2<f32>(1.0));
    return out;
}

// Distance from `p` to a box of half-size `half` with corners of radius
// `r`, centred on the origin: negative inside.
fn rounded_box(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn clipped(pos: vec2<f32>, clip: vec4<f32>) -> bool {
    return pos.x < clip.x || pos.y < clip.y || pos.x >= clip.x + clip.z || pos.y >= clip.y + clip.w;
}

// Coverage of the box at this pixel, and of its inside past the border.
fn coverage(pos: vec2<f32>, rect: vec4<f32>, radius: f32, border: f32) -> vec2<f32> {
    let half = rect.zw * 0.5;
    let r = min(radius, min(half.x, half.y));
    let d = rounded_box(pos - (rect.xy + half), half, r);
    return vec2<f32>(clamp(0.5 - d, 0.0, 1.0), clamp(0.5 - (d + border), 0.0, 1.0));
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    if clipped(in.pos.xy, in.clip) {
        discard;
    }
    let cover = coverage(in.pos.xy, in.rect, in.params.x, in.params.y);
    // Premultiplied, so that a transparent fill inside an opaque border
    // does not darken the edge between them.
    let fill = vec4<f32>(in.fill.rgb * in.fill.a, in.fill.a);
    let border = vec4<f32>(in.border.rgb * in.border.a, in.border.a);
    var color = fill;
    if in.params.y > 0.0 {
        color = mix(border, fill, cover.y);
    }
    return color * cover.x;
}

// --- pictures ---------------------------------------------------------

@group(1) @binding(0) var picture: texture_2d<f32>;
@group(1) @binding(1) var picture_sampler: sampler;

// A picture's texture is sRGB, so sampling gives linear light; the target
// holds sRGB values, so it goes back.
fn encode_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(high, low, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_picture(in: Out) -> @location(0) vec4<f32> {
    // Sampled before anything branches: a sample needs uniform control
    // flow for its derivatives.
    let uv = clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let texel = textureSample(picture, picture_sampler, uv);
    if clipped(in.pos.xy, in.clip) {
        discard;
    }
    let cover = coverage(in.pos.xy, in.rect, in.params.x, 0.0);
    let alpha = texel.a * in.fill.a * cover.x;
    return vec4<f32>(encode_srgb(texel.rgb) * alpha, alpha);
}
