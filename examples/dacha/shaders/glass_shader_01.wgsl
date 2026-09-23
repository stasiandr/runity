// From Assets/Content/Art/Materials/Glass_Shader_01.shadergraph, values from
// M_Glass.mat.
//
// The original is a plain URP Lit transparent graph: base colour, smoothness
// (from a property named _Roughness) and a constant opacity, metallic 0, alpha
// clip at 0.5.
//
// This port sets the same values. Nothing is left out of the graph, but
// M_Glass.mat says _Surface 0, so the importer may bring it in as opaque and
// ignore `alpha`; the alpha clip at 0.5 is not repeated (0.54 passes it).

const GLASS_SHADER_01_COLOR = vec3<f32>(0.31372374, 0.39137214, 0.4716981);
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
