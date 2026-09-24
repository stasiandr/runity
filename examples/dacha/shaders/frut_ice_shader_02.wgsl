// From Assets/Content/Art/Materials/Frut_Ice_Shader_02.shadergraph, values
// from MI_Frut_Ice_02.mat; _Ice_Int and _Ice_Color come from the material
// (M_Water_01.mat uses it too). The colour is sRGB in the .mat and made
// linear here, as Unity does.
// scrap:params _Ice_Int _Ice_Color.r _Ice_Color.g _Ice_Color.b
// scrap:textures _SampleTexture2D_7d72135f9d74416ea46068c3d7433d49_Texture_1_Texture2D _SampleTexture2D_057c6fd260f848edb0f7231e4a6fa5c3_Texture_1_Texture2D _SampleTexture2D_6ad4c1740f58419ca8745a03719c808e_Texture_1_Texture2D
//
// The original is an unlit transparent graph: alpha is a frost texture
// (T_Freeze__01_M) plus _Ice_Int; colour is _Ice_Color plus glints where a
// sparse dot texture (T_Noise_Glimmer_01_M) is bright both in the mesh's UVs
// and in screen space, faded by a hard-contrast scene-depth term.
//
// This port reads both textures from the material. M_Water_01 is a variant
// of MI_Frut_Ice_02 and its .scrmat lacks the frost texture, so there the
// frost reads white and the water is opaque. Scene depth (what is behind
// the ice) is replaced by the fragment's own distance with the camera's
// 1000 m far plane, and clamped at 0 (the original goes negative there,
// darkening far glints). Unlit is imitated with a black albedo and the
// colour in `emission`.

const FRUT_ICE_SHADER_02_ICE_TILING = 1.0;
const FRUT_ICE_SHADER_02_SCREEN_TILING = 0.71;
const FRUT_ICE_SHADER_02_FAR = 1000.0;

fn frut_ice_shader_02_srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;

    // Tiled about Unity's uv origin, bottom left; scrap's v runs down.
    let ice_uv = vec2<f32>(in.uv.x, 1.0 - in.uv.y) * FRUT_ICE_SHADER_02_ICE_TILING;
    let frost = texture_at(in, 0u, vec2<f32>(ice_uv.x, 1.0 - ice_uv.y)).r;
    o.alpha = clamp(frost + in.params[0].x, 0.0, 1.0);

    // Glimmer in UV: step(1, r * 3).
    let uv_glint = step(1.0, texture_at(in, 1u, in.uv).r * 3.0);
    // Glimmer in screen space (Unity's, origin bottom left) at 0.71
    // tiling: step(1, r).
    let clip = frame.view_projection * vec4<f32>(in.world_position, 1.0);
    let screen = (clip.xy / clip.w * 0.5 + 0.5) * FRUT_ICE_SHADER_02_SCREEN_TILING;
    let screen_glint = step(1.0, texture_at(in, 2u, vec2<f32>(screen.x, 1.0 - screen.y)).r);

    // 1 - Contrast(Linear01Depth * 100, 100).
    let depth01 = length(frame.camera_position.xyz - in.world_position) / FRUT_ICE_SHADER_02_FAR;
    let midpoint = pow(0.5, 2.2);
    let near_fade = max(1.0 - ((depth01 * 100.0 - midpoint) * 100.0 + midpoint), 0.0);

    let color = frut_ice_shader_02_srgb_to_linear(in.params[0].yzw) + vec3<f32>(uv_glint * screen_glint * near_fade);
    o.albedo = vec3<f32>(0.0);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = color;
    return o;
}
