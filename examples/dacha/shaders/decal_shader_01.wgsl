// Decal: a painted number on the plots.
// From Assets/Content/Art/Materials/Decal_Shader_01.shadergraph (M_Decal_01..06).
// A URP decal: flat _Color, alpha from the red of a mask texture (T_Num_0N_M,
// a digit) times _Mask_int (5.67).
// Here it is a surface on whatever mesh carries it, not a projector. The mask
// cannot be sampled on its own, so it is read from the material's base map:
// give the material the digit picture as base map with a white colour. With
// no base map the whole face is painted. Alpha only shows if the material is
// transparent.

const DECAL_COLOR = vec3<f32>(0.4452, 0.2929, 0.2049);
const DECAL_MASK_INT = 5.67;

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let mask = out.albedo.r;
    o.albedo = DECAL_COLOR;
    o.alpha = out.alpha * clamp(mask * DECAL_MASK_INT, 0.0, 1.0);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = vec3<f32>(0.0);
    return o;
}
