// From Assets/Content/Art/Materials/Atlas_Shader_Grab_01.shadergraph (URP Lit).
// A flat colour pulsing in brightness: Color * clamp(sin(time * Speed), 0.3, 1),
// smoothness 0.
// Port: the colour and speed are M_Grab's (M_Blink_01: colour sRGB
// 1, 0.614, 0.325 at speed 9; M_Blink_02: 1, 0.436, 0.401 at speed 8). The
// importer gives these materials their _BaseColor, not the graph's _Color,
// so the albedo from the material is ignored here.

const GRAB_COLOR: vec3<f32> = vec3<f32>(0.3813, 0.0663, 0.0663); // sRGB 0.651, 0.286, 0.286
const GRAB_SPEED: f32 = 5.0;

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    o.albedo = GRAB_COLOR * clamp(sin(in.time * GRAB_SPEED), 0.3, 1.0);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    return o;
}
