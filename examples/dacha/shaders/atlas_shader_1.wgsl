// From Assets/Content/Art/Materials/Atlas_Shader 1.shadergraph (URP Lit).
// The atlas colour (UV0) overlaid with a greyscale grunge texture
// (_Triplanar_Texture, JB_HPBase_FlameRake_BW) projected triplanar at 25%;
// optionally lerped toward a second triplanar texture (_Up_Vector_Texture)
// on tops ("Up Vecto?") and a faint near-camera pulse in emission ("Blink?").
// Port: the atlas is the material's base map (the graph's Sample Texture 2D
// holds SM_Refrigerator_1_TXTR, the same picture as the base map
// T_SM_Refrigerator_1_TXTR). Both triplanar textures are read from the
// material with the graph's tiling and blend; the projection uses world
// position, not object position (Translate and Rotation are zero in every
// material, so only that differs). The blink's scene-depth falloff is
// dropped. The two toggles come from the material (M_Atlas_Triplanar_01 1
// has Up Vecto?, with T_Road_01_D_2 as its up texture;
// M_Atlas_Triplanar_Blink_01 has Blink?); the other values are the graph's
// defaults, which the three materials share. An unset up texture reads
// white, as in Unity.
// runity:params _Up_Vecto _Blink
// runity:textures _Triplanar_Texture _Up_Vector_Texture

const ATLAS_TILING: f32 = 0.31;
const ATLAS_BLEND: f32 = 1.0;
const ATLAS_TRIPLANAR_INT: f32 = 0.25;
const ATLAS_SMOOTHNESS: f32 = 0.426; // _Roughtnes, wired to Smoothness

fn atlas_overlay(base: vec3<f32>, blend: vec3<f32>) -> vec3<f32> {
    let low = 2.0 * base * blend;
    let high = 1.0 - 2.0 * (1.0 - base) * (1.0 - blend);
    return select(high, low, base <= vec3<f32>(0.5));
}

// Unity's Triplanar node: the texture seen along each axis, weighted by
// how much the normal faces it.
fn atlas_triplanar(in: SurfaceIn, slot: u32, p: vec3<f32>, w: vec3<f32>) -> vec3<f32> {
    let x = texture_at(in, slot, p.zy).rgb;
    let y = texture_at(in, slot, p.xz).rgb;
    let z = texture_at(in, slot, p.xy).rgb;
    return x * w.x + y * w.y + z * w.z;
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let p = in.world_position * ATLAS_TILING;
    var w = pow(abs(in.normal), vec3<f32>(ATLAS_BLEND));
    w = w / max(w.x + w.y + w.z, 1e-5);
    // Both projections are read on every pixel: a texture is read where
    // all pixels run together, and the toggle below only picks.
    let grunge = atlas_triplanar(in, 0u, p, w);
    let up_texture = atlas_triplanar(in, 1u, p, w);
    var color = mix(out.albedo, atlas_overlay(out.albedo, grunge), ATLAS_TRIPLANAR_INT);
    if in.params[0].x > 0.5 {
        let mid = pow(0.5, 2.2);
        let up = clamp(dot(in.normal, vec3<f32>(0.0, 1.0, 0.0)), 0.0, 1.0);
        let t = ((up - mid) * 2.73 + mid) * 0.2;
        color = mix(color, up_texture, t);
    }
    o.albedo = color;
    o.metallic = 0.0;
    o.smoothness = ATLAS_SMOOTHNESS;
    if in.params[0].y > 0.5 {
        o.emission = vec3<f32>(clamp(sin(in.time / 3.0 * 6.0) / 30.0, 0.0, 1.0));
    } else {
        o.emission = vec3<f32>(0.0);
    }
    return o;
}
