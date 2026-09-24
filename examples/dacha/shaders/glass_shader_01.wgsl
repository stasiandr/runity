// From Assets/Content/Art/Materials/Glass_Shader_01.shadergraph, values from
// M_Glass.mat.
//
// The original is a plain URP Lit transparent graph: base colour, smoothness
// (from a property named _Roughness) and a constant opacity, metallic 0, alpha
// clip at 0.5.
//
// This port sets the same values; nothing is left out of the graph. The
// alpha clip at 0.5 is the importer's (0.54 passes it).

const GLASS_SHADER_01_COLOR = vec3<f32>(0.08022, 0.1269, 0.18877); // sRGB 0.314, 0.391, 0.472
const GLASS_SHADER_01_SMOOTHNESS = 0.65;
const GLASS_SHADER_01_OPACITY = 0.54;

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    o.albedo = GLASS_SHADER_01_COLOR;
    o.metallic = 0.0;
    o.smoothness = GLASS_SHADER_01_SMOOTHNESS;
    o.alpha = GLASS_SHADER_01_OPACITY;
    return o;
}
