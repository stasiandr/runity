// Grass: grass, bush and tumbleweed cards.
// From Assets/Content/_Incoming/Grass.shadergraph (M_Grass_Model_01..03,
// M_Bush_01w, MI_Tumbleweed_01). The _baza texture packs a height gradient in
// red (lerping _Color to _Color_2) and the blade shape in green (alpha, clipped
// at 0.5); in the vertex stage the mesh sways along x with gradient noise over
// world position and time, more towards the top of the UVs.
// Here _baza is read from the material by UV0 (each material its own:
// T_Grass_Mesh_01..04, T_Tumbleweed_01_M): red picks the colour, green is cut
// at 0.5 with discard. _Color and _Color_2 (sRGB) come from the material. The
// wind sway is the engine's own (the material's `wind`, which also tramples
// it, and `translucency` for the light through the blades). Shadows are
// cut by the base map's alpha (the importer makes _baza the base map), not by
// its green, as the shadow pass does not run this function; the _baza
// textures have no alpha of their own, so theirs is their green
// (`alpha_from: Some(Green)` in each one's .scrimport) — or a card's shadow
// is its whole quad.
// scrap:params _Color.r _Color.g _Color.b _Color_2.r _Color_2.g _Color_2.b
// scrap:textures _baza
// scrap:material wind: 1.5, translucency: 0.4

const GRASS_CLIP = 0.5;

fn grass_srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let baza = texture_at(in, 0u, in.uv);
    if baza.g < GRASS_CLIP {
        discard;
    }
    let color = grass_srgb_to_linear(in.params[0].xyz);
    let color_2 = grass_srgb_to_linear(vec3<f32>(in.params[0].w, in.params[1].xy));
    o.albedo = mix(color, color_2, baza.r);
    o.alpha = out.alpha * baza.g;
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = vec3<f32>(0.0);
    return o;
}
