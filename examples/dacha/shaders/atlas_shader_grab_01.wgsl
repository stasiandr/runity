// From Assets/Content/Art/Materials/Atlas_Shader_Grab_01.shadergraph (URP Lit).
// A flat colour pulsing in brightness: Color * clamp(sin(time * Speed), 0.3, 1),
// smoothness 0.
// Port: _Color (sRGB) and _Speed come from the material. The importer gives
// these materials their _BaseColor, not the graph's _Color, so the albedo
// from the material is ignored here.
// runity:params _Color.r _Color.g _Color.b _Speed

fn grab_srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let color = grab_srgb_to_linear(in.params[0].xyz);
    o.albedo = color * clamp(sin(in.time * in.params[0].w), 0.3, 1.0);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    return o;
}
