// Huge sand: the far dunes.
// From Assets/Content/Art/Materials/Huge_Sand_Shader.shadergraph (M_Huge_Sand_01).
// The original blends the flat colour _Color towards the T_Huge_Sand_01
// texture (by _Color_Int = 0.36, on the mesh's UVs), and in the vertex stage
// lifts the mesh by a slowly scrolling noise texture.
// Here the sand texture is the material's own, on the same UVs. The vertex
// lift is left out: a surface function cannot move vertices. Smoothness,
// metallic and emission are the graph's own (all zero), not the .mat's.
// runity:textures _SampleTexture2D_27a0de0802834c909c9bf906c47b3516_Texture_1_Texture2D

const HUGE_SAND_TEXTURE = 0u;
const HUGE_SAND_COLOR = vec3<f32>(0.2946, 0.149, 0.0925);
const HUGE_SAND_COLOR_INT = 0.36;

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let sand = texture_at(in, HUGE_SAND_TEXTURE, in.uv).rgb;
    o.albedo = mix(HUGE_SAND_COLOR, sand, HUGE_SAND_COLOR_INT);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = vec3<f32>(0.0);
    return o;
}
