// Landscape: the ground around the dacha.
// From Assets/Content/Art/Materials/Landscape_Shader_01.shadergraph (M_Landscape_01).
// The original lerps two road textures (T_Road_01_D_1 at UV x40, T_Road_01_D_2
// at x60, the second graded by contrast and saturation) by vertex colour red,
// then by the red of T_Sand_01_T_Noise_01 (x5, times _Multi) towards a
// differently graded second road, then by the red of T_Road_01_Noise_Animate
// (x5, scrolling in v with time, times _Multi_Anim) towards a stronger grade
// of the result; emission is sparse glimmer specks from T_Noise_Glimmer_01_M
// (x141.5), masked by the same texture laid over the screen and faded out
// with scene depth near the camera.
// Here the road and both noise textures are the material's own, read as the
// graph reads them, and vertex colour red is the mesh's own
// (`in.vertex_color`, the FBX's colours as Unity reads them). The glimmer texture would be a fifth, so its specks are
// procedural: a rare lit cell in a fine grid on the UVs, twinkling with time
// where the original twinkles as the screen-space mask slides over it; the
// depth fade is left out (no scene depth or camera here). The final
// hue/saturation/contrast pass is identity at the material's values and is
// skipped. Metallic is the graph's 0, not the .mat's _Metallic = 1; the .mat's
// base map (a stale refrigerator texture) is ignored.
// scrap:textures _SampleTexture2D_5f1339acdb164c14b236042c20de1ef1_Texture_1_Texture2D _SampleTexture2D_353fddecd0514a679b12f6c2ebe27635_Texture_1_Texture2D _SampleTexture2D_54c0ff5071964b64bf228a86c4406d03_Texture_1_Texture2D _SampleTexture2D_94a9ef748c05450ebe18c4ea0db054a7_Texture_1_Texture2D

const LANDSCAPE_ROAD_1 = 0u;
const LANDSCAPE_ROAD_2 = 1u;
const LANDSCAPE_NOISE = 2u;
const LANDSCAPE_NOISE_ANIM = 3u;

const LANDSCAPE_TILING = 40.0;
const LANDSCAPE_TILING_1 = 5.0;
const LANDSCAPE_TILING_3 = 60.0;
const LANDSCAPE_CONTRAST_01 = 2.55;
const LANDSCAPE_CONTRAST_02 = 1.71;
const LANDSCAPE_CONTRAST_ANIM = 1.34;
const LANDSCAPE_SATURATION = 0.62;
const LANDSCAPE_SATURATION_2 = 0.68;
const LANDSCAPE_SATURATION_ANIM = 0.83;
const LANDSCAPE_MULTI = 2.39;
const LANDSCAPE_MULTI_ANIM = 12.0;
const LANDSCAPE_SPEED = 0.24;
const LANDSCAPE_GLIMMER_TILING = 141.5;
const LANDSCAPE_GLIMMER_INT = 3.0;
const LANDSCAPE_ROUGHTNES = 0.054;

fn landscape_hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(123.34, 456.21));
    let r = q + dot(q, q + 45.32);
    return fract(r.x * r.y);
}

// Shader Graph's Contrast and Saturation nodes.
fn landscape_contrast(c: vec3<f32>, k: f32) -> vec3<f32> {
    let mid = pow(0.5, 2.2);
    return (c - mid) * k + mid;
}

fn landscape_saturation(c: vec3<f32>, s: f32) -> vec3<f32> {
    let luma = dot(c, vec3<f32>(0.2126729, 0.7151522, 0.072175));
    return luma + s * (c - luma);
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let uv = in.uv;

    let road_1 = texture_at(in, LANDSCAPE_ROAD_1, uv * LANDSCAPE_TILING).rgb;
    let road_2 = texture_at(in, LANDSCAPE_ROAD_2, uv * LANDSCAPE_TILING_3).rgb;
    let noise = texture_at(in, LANDSCAPE_NOISE, uv * LANDSCAPE_TILING_1).r;
    let anim_uv = uv * LANDSCAPE_TILING_1 + vec2<f32>(0.0, in.time * LANDSCAPE_SPEED);
    let noise_anim = texture_at(in, LANDSCAPE_NOISE_ANIM, anim_uv).r;

    let painted = in.vertex_color.r;
    let graded_1 = landscape_saturation(landscape_contrast(road_2, LANDSCAPE_CONTRAST_01), LANDSCAPE_SATURATION);
    let c1 = mix(road_1, graded_1, painted);

    // The lerps are unclamped, as the graph's are: past 1 they overshoot.
    let graded_2 = landscape_saturation(landscape_contrast(road_2, LANDSCAPE_CONTRAST_02), LANDSCAPE_SATURATION_2);
    let c2 = mix(c1, graded_2, noise * LANDSCAPE_MULTI);

    let graded_anim = landscape_saturation(landscape_contrast(c2, LANDSCAPE_CONTRAST_ANIM), LANDSCAPE_SATURATION_ANIM);
    let c3 = mix(c2, graded_anim, noise_anim * LANDSCAPE_MULTI_ANIM);

    // Glimmer: rare specks in a fine grid, each lit now and then — a
    // pixel or two across whatever the mesh's UVs make of the grid, as the
    // texture's specks are: a sand mesh whose UVs spread far drew a cell a
    // metre wide, and its speck as a white blot on the ground.
    let g = uv * LANDSCAPE_GLIMMER_TILING;
    let cell = floor(g);
    let chance = landscape_hash(cell);
    let pixels = length(fract(g) - 0.5) / max(length(fwidth(g)), 1e-6);
    let spot = 1.0 - smoothstep(0.5, 1.5, pixels);
    let speck = step(0.97, chance) * spot * min(LANDSCAPE_GLIMMER_INT, 1.0);
    let twinkle = step(0.7, sin(in.time * 3.0 + chance * 60.0));

    o.albedo = max(c3, vec3<f32>(0.0));
    o.metallic = 0.0;
    o.smoothness = LANDSCAPE_ROUGHTNES;
    o.emission = vec3<f32>(speck * twinkle);
    return o;
}
