// Decal: a painted number on the plots.
// From Assets/Content/Art/Materials/Decal_Shader_01.shadergraph (M_Decal_01..06).
// A URP decal: flat _Color, alpha from the red of _Decal_Texture (T_Num_0N_M,
// a digit, sampled at mip 0 by UV0) times _Mask_int (5.67).
// Here it is a surface on whatever mesh carries it, not a projector, and the
// digit is read by the mesh's UVs, mipmapped. Each material binds its own
// digit; without one the whole face is painted. _Color and _Mask_int are the
// same in every material, so they are constants. Alpha only shows if the
// material is transparent (the six are opaque in Unity, where the decal
// projector blends regardless).
// scrap:textures _Decal_Texture

const DECAL_COLOR = vec3<f32>(0.4452, 0.2929, 0.2049);
const DECAL_MASK_INT = 5.67;

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let mask = texture_at(in, 0u, in.uv).r;
    o.albedo = DECAL_COLOR;
    o.alpha = out.alpha * clamp(mask * DECAL_MASK_INT, 0.0, 1.0);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = vec3<f32>(0.0);
    return o;
}
