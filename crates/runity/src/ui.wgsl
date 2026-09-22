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
};

struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
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
    return out;
}

@fragment
fn fs(in: Output) -> @location(0) vec4<f32> {
    return in.color;
}
