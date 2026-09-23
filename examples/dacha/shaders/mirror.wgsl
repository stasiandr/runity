// mirror: from Assets/Content/Mirror/Shaders/Mirror.shader, the surface half
// of Dacha's PlanarReflectionMirror. The original samples the reflection
// camera's texture by screen position and blends it over the tint by
// strength, unlit.
//
// Here the reflection is runity's planar mirror: the importer turns the
// PlanarReflectionMirror component into `render_texture: (name: "mirror",
// mirror: true)`, and a material on this shader shows that picture laid on
// the screen (the two lines below). Tint and strength come from the
// material. Two mirrors in view share one picture.
//
// runity:params _Tint.r _Tint.g _Tint.b _Strength
// runity:base_map render:mirror
// runity:screen_map Mirror

fn mirror_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let tint = mirror_to_linear(in.params[0].xyz);
    let strength = in.params[0].w;
    o.albedo = mix(tint, out.albedo * tint, strength);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    return o;
}
