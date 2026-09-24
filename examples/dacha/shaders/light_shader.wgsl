// From Assets/Content/Art/Materials/Light_Shader.shadergraph (URP Sprite
// Unlit, both faces). Values from M_Light.mat; _Color_Int and _Alpha come
// from the material (M_Light 1 has 0.3 and 0.3).
// runity:params _Color_Int _Alpha
// runity:textures _SampleTexture2D_666ec71047f14411af8909bca7478ba4_Texture_1_Texture2D _SampleTexture2D_d1e16623bfa34629bae7ce91e3854ab1_Texture_1_Texture2D
//
// A fake light beam: _Color x _Color_Int, with alpha = T_SunLines_M's red
// x _Alpha, minus a second, stretched copy of the same streaks scrolling
// slowly upwards, so the beam shimmers.
//
// This port reads T_SunLines_M from the material, as the graph does. The
// colour goes to emission over a black albedo; _Color is the .mat's, fixed
// here.

const LIGHT_COLOR: vec3<f32> = vec3<f32>(1.0, 0.6444, 0.4126); // sRGB 1.0, 0.824, 0.675
const LIGHT_TILING: vec2<f32> = vec2<f32>(0.67, 0.37);
const LIGHT_SPEED: f32 = 0.01;

// Unity's v runs up the texture; runity's runs down.
fn light_flip(uv: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(uv.x, 1.0 - uv.y);
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let uv = light_flip(in.uv);
    let beam = texture_at(in, 0u, in.uv).r * in.params[0].y;
    let moving = texture_at(in, 1u, light_flip(uv * LIGHT_TILING + vec2<f32>(0.0, LIGHT_SPEED * in.time))).r;
    o.albedo = vec3<f32>(0.0);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = LIGHT_COLOR * in.params[0].x;
    o.alpha = clamp(beam - moving, 0.0, 1.0);
    return o;
}
