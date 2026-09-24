// Sand decal: a patch of loose sand or grass laid over the ground.
// From Assets/Content/Art/Materials/Sand_Decal_01.shadergraph (M_Sand_Decal_01,
// M_Green_Decal_01). A URP decal: colour from _Texture2D (T_Decal_Sand_01 or
// T_Grass_01_D) at UV times _Tiling (1.55), alpha from the red of the
// T_Sand_Decal_01_M mask on the plain UVs times _Mask_int (1).
// Here both textures are the material's own, read as the graph reads them,
// but it is a surface on whatever mesh carries it, not a projector. Alpha
// only shows if the material is transparent.
// scrap:textures _Texture2D _SampleTexture2D_6df76a1b04f04a72824c5069c4db43d9_Texture_1_Texture2D
// scrap:params _Tiling _Mask_int

const SAND_DECAL_COLOR = 0u;
const SAND_DECAL_MASK = 1u;

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let tiling = in.params[0].x;
    let mask_int = in.params[0].y;
    o.albedo = texture_at(in, SAND_DECAL_COLOR, in.uv * tiling).rgb;
    o.alpha = texture_at(in, SAND_DECAL_MASK, in.uv).r * mask_int;
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = vec3<f32>(0.0);
    return o;
}
