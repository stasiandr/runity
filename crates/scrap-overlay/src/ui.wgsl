// Filled rectangles in pixel coordinates. Four vertices generated from the
// vertex index, so a quad costs one instance and no vertex buffer of its own.
// Each is its colour times its picture (a white pixel for a plain one).

struct Screen {
    // width, height, and two words of padding.
    size: vec4<f32>,
};

@group(0) @binding(0) var<uniform> screen: Screen;
@group(1) @binding(0) var picture: texture_2d<f32>;
@group(1) @binding(1) var picture_sampler: sampler;

struct Instance {
    @location(0) rect: vec4<f32>,
    @location(1) color: vec4<f32>,
    // The corner radius in pixels, the edge's softness in pixels, the
    // outline's thickness (0 filled), the share swept clockwise from twelve.
    @location(2) shape: vec4<f32>,
    // The part of its picture: left, top, right, bottom, 0 to 1.
    @location(3) uv: vec4<f32>,
    // The colour at the bottom edge (the same as `color` for a flat one).
    @location(4) color_bottom: vec4<f32>,
};

struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    // Where in the quad, in pixels from its middle; its half size; radius.
    @location(1) local: vec2<f32>,
    @location(2) half: vec2<f32>,
    @location(3) radius: f32,
    @location(4) uv: vec2<f32>,
    // Softness, stroke, sweep.
    @location(5) shape: vec3<f32>,
};

@vertex
fn vs(@builtin(vertex_index) index: u32, instance: Instance) -> Output {
    // A triangle strip's four corners, in the order (0,0) (1,0) (0,1) (1,1).
    let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
    let pixel = instance.rect.xy + corner * instance.rect.zw;

    // Pixels from the top left to clip space, which runs -1..1 with y up.
    let ndc = vec2<f32>(
        pixel.x / screen.size.x * 2.0 - 1.0,
        1.0 - pixel.y / screen.size.y * 2.0,
    );

    var out: Output;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.color = mix(instance.color, instance.color_bottom, corner.y);
    out.half = instance.rect.zw * 0.5;
    out.local = (corner - vec2<f32>(0.5)) * instance.rect.zw;
    out.radius = min(instance.shape.x, min(out.half.x, out.half.y));
    out.uv = mix(instance.uv.xy, instance.uv.zw, corner);
    out.shape = instance.shape.yzw;
    return out;
}

@fragment
fn fs(in: Output) -> @location(0) vec4<f32> {
    let color = in.color * textureSample(picture, picture_sampler, in.uv);
    let soft = in.shape.x;
    let stroke = in.shape.y;
    let sweep = in.shape.z;
    if in.radius <= 0.0 && soft <= 0.0 && stroke <= 0.0 && sweep >= 1.0 {
        return color;
    }
    // A rounded box's distance (negative inside), and a pixel of softness
    // at its edge — or `soft` pixels of it, inward.
    let q = abs(in.local) - in.half + vec2<f32>(in.radius);
    let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - in.radius;
    var a = clamp(0.5 - d, 0.0, 1.0);
    if soft > 0.0 {
        a = clamp(-d / soft, 0.0, 1.0);
        a = a * a * (3.0 - 2.0 * a);
    }
    if stroke > 0.0 {
        // Only the band `stroke` wide inside the edge.
        a = a * clamp(d + stroke + 0.5, 0.0, 1.0);
    }
    if sweep < 1.0 {
        // Clockwise from twelve o'clock, y down: the angle of this pixel
        // round the middle, 0 to 1, against the share; a pixel's softness.
        let turn = atan2(in.local.x, -in.local.y) / 6.2831853;
        let at = fract(turn + 1.0);
        let px = max(length(in.local), 1.0) * 6.2831853;
        a = a * clamp((sweep - at) * px + 0.5, 0.0, 1.0);
    }
    return vec4<f32>(color.rgb, color.a * a);
}
