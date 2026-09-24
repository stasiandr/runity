// Filled rectangles in pixel coordinates. Four vertices generated from the
// vertex index, so a quad costs one instance and no vertex buffer of its own.

struct Screen {
    // width, height, and two words of padding.
    size: vec4<f32>,
};

@group(0) @binding(0) var<uniform> screen: Screen;

struct Instance {
    @location(0) rect: vec4<f32>,
    @location(1) color: vec4<f32>,
    // The corner radius in pixels, and three words of padding.
    @location(2) shape: vec4<f32>,
};

struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    // Where in the quad, in pixels from its middle; its half size; radius.
    @location(1) local: vec2<f32>,
    @location(2) half: vec2<f32>,
    @location(3) radius: f32,
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
    out.color = instance.color;
    out.half = instance.rect.zw * 0.5;
    out.local = (corner - vec2<f32>(0.5)) * instance.rect.zw;
    out.radius = min(instance.shape.x, min(out.half.x, out.half.y));
    return out;
}

@fragment
fn fs(in: Output) -> @location(0) vec4<f32> {
    if in.radius <= 0.0 {
        return in.color;
    }
    // A rounded box's distance, and a pixel of softness at its edge.
    let q = abs(in.local) - in.half + vec2<f32>(in.radius);
    let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - in.radius;
    let a = clamp(0.5 - d, 0.0, 1.0);
    return vec4<f32>(in.color.rgb, in.color.a * a);
}
