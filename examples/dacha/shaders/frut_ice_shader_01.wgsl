// From Assets/Content/Art/Materials/Frut_Ice_Shader_01.shadergraph, values
// from MI_Frut_Ice.mat (also used by MI_Frut_Ice 1 and 2, which differ only in
// _Main_Color).
//
// The original is an opaque URP Lit graph: a flat _Main_Color, smoothness 0.5,
// and a vertex stage that scales the object by _Scale plus a pulse
// sin(2t) * _Puls_Int.
//
// The vertex scale/pulse is dropped (the surface function cannot move
// vertices; _Puls_Int is 0 in every material, so only the static _Scale 1.45
// is lost). The importer does not read _Main_Color, so when it hands over
// plain white this uses MI_Frut_Ice's colour; any other colour is kept.
// The ice texture slots in the .mat files are left over from an older graph
// and unused.

const FRUT_ICE_SHADER_01_MAIN_COLOR = vec3<f32>(0.62352943, 0.19607843, 0.24556275);
const FRUT_ICE_SHADER_01_SMOOTHNESS = 0.5;

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    if all(out.albedo > vec3<f32>(0.999)) {
        o.albedo = FRUT_ICE_SHADER_01_MAIN_COLOR;
    }
    o.metallic = 0.0;
    o.smoothness = FRUT_ICE_SHADER_01_SMOOTHNESS;
    return o;
}
