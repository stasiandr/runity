// scrap-ui: rounded rectangles and pictures.
//
// Everything is in physical pixels from the top left. Colours arrive as the
// design file writes them — sRGB, straight alpha — and the target is the
// plain-bytes view of an sRGB texture, so blending happens on sRGB values,
// as in a browser. The fragment works out the distance to the rounded box
// and fades over one pixel, which is the anti-aliasing; the border is the
// band within `border_width` of the edge.
//
// A drawing's pieces share the pipeline, told apart by `params.z`: 0 is
// the box above, 1 a straight piece of line with round ends from `rect.xy`
// to `rect.zw`, `params.x` thick on each side, and 2 a box turned along
// that piece — a diamond is a square turned by 45 degrees. They are
// interleaved with the boxes in one draw, so a drawing stays under the
// boxes painted after it.

struct Screen {
    size: vec2<f32>,
    _pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> screen: Screen;

struct Shape {
    @location(0) rect: vec4<f32>,
    @location(1) fill: vec4<f32>,
    @location(2) border: vec4<f32>,
    // radius (half thickness for a line), border width, kind, unused
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
    var p = shape.rect.xy - vec2<f32>(grow) + corner * (shape.rect.zw + vec2<f32>(2.0 * grow));
    if shape.params.z > 0.5 {
        let a = shape.rect.xy;
        let b = shape.rect.zw;
        let axis = along(a, b);
        let half_width = shape.params.x;
        // A line's round ends reach past its points by half its width.
        let ends = select(0.0, half_width, shape.params.z < 1.5);
        let reach = 0.5 * length(b - a) + ends + grow;
        let side = (corner * 2.0 - vec2<f32>(1.0)) * vec2<f32>(reach, half_width + grow);
        p = 0.5 * (a + b) + axis * side.x + vec2<f32>(-axis.y, axis.x) * side.y;
    }
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

// The unit direction from `a` to `b`, or across for a piece of no length:
// a dot.
fn along(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let d = b - a;
    let len = length(d);
    if len < 1.0e-4 {
        return vec2<f32>(1.0, 0.0);
    }
    return d / len;
}

// Distance from `p` to a drawing's piece: a line with round ends, or a
// box turned along it.
fn piece(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, half_width: f32, kind: f32) -> f32 {
    if kind < 1.5 {
        let pa = p - a;
        let ba = b - a;
        let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1.0e-8), 0.0, 1.0);
        return length(pa - ba * h) - half_width;
    }
    let axis = along(a, b);
    let m = p - 0.5 * (a + b);
    let local = vec2<f32>(dot(m, axis), dot(m, vec2<f32>(-axis.y, axis.x)));
    return rounded_box(local, vec2<f32>(0.5 * length(b - a), half_width), 0.0);
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
    var cover = coverage(in.pos.xy, in.rect, in.params.x, in.params.y);
    if in.params.z > 0.5 {
        let d = piece(in.pos.xy, in.rect.xy, in.rect.zw, in.params.x, in.params.z);
        cover = vec2<f32>(clamp(0.5 - d, 0.0, 1.0), clamp(0.5 - (d + in.params.y), 0.0, 1.0));
    }
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
