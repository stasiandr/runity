// From Assets/Content/Art/Materials/Energy_Cell_Shader.shadergraph (URP Lit,
// opaque, both faces). Values from M_Energy_Cell_01.mat.
// scrap:textures _SampleTexture2D_27a0de0802834c909c9bf906c47b3516_Texture_1_Texture2D
//
// The original colours the cell with _Color mixed 10% towards a sand
// texture (T_Huge_Sand_01), emits that colour x4, and pushes the vertices
// by a scrolling noise texture (_Displacement 0.01).
//
// This port reads T_Huge_Sand_01 from the material; the vertex
// displacement is dropped (there is no vertex hook).

const ENERGY_CELL_COLOR: vec3<f32> = vec3<f32>(1.0, 0.3313, 0.0673); // sRGB 1.0, 0.611, 0.288
const ENERGY_CELL_COLOR_INT: f32 = 0.1;
const ENERGY_CELL_EMISSIVE_INT: f32 = 4.0;

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let sand = texture_at(in, 0u, in.uv).rgb;
    let base = mix(ENERGY_CELL_COLOR, sand, ENERGY_CELL_COLOR_INT);
    o.albedo = base;
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = base * ENERGY_CELL_EMISSIVE_INT;
    return o;
}
