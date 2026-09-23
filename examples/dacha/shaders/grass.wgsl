// Grass: grass, bush and tumbleweed cards.
// From Assets/Content/_Incoming/Grass.shadergraph (M_Grass_Model_01..03,
// M_Bush_01w, MI_Tumbleweed_01). The _baza texture packs a height gradient in
// red (lerping _Color to _Color_2) and the blade shape in green (alpha, clipped
// at 0.5); in the vertex stage the mesh sways along x with gradient noise over
// world position and time, more towards the top of the UVs.
// Here _baza is read from the material's base map (give it that texture with a
// white colour): red picks the colour, green is cut at 0.5 with discard. The
// colours are M_Grass_Model_01's. The wind sway is left out: a surface
// function cannot move vertices. Shadows still fall from the whole card, as
// the shadow pass only knows the base map's alpha.

const GRASS_COLOR = vec3<f32>(0.2552, 0.3303, 0.1578);
const GRASS_COLOR_2 = vec3<f32>(0.7615, 0.7432, 0.0249);
const GRASS_CLIP = 0.5;

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let baza = out.albedo;
    if baza.g < GRASS_CLIP {
        discard;
    }
    o.albedo = mix(GRASS_COLOR, GRASS_COLOR_2, baza.r);
    o.alpha = out.alpha * baza.g;
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = vec3<f32>(0.0);
    return o;
}
