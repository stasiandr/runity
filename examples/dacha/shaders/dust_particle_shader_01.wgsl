// From Assets/Content/Art/Materials/Dust_Particle_Shader_01.shadergraph (URP Lit).
// A lit dust puff: flat Color for albedo, alpha from the red channel of a
// cloud-shaped mask (T_Dust_01_M), cut out at 0.5.
// runity:textures _Alpha
// Port: the mask is read from the material's _Alpha and cut out here with
// a discard. The albedo is Color alone, as in the graph (neither the
// particle's colour nor the base map, the same mask, tints it). The
// importer also sets alpha_clip 0.5, which runity applies before this
// function to its own alpha: the particle's fade (the mask has no alpha
// channel), so a puff fading below half vanishes whole, where Unity's
// graph ignores the fade. The shadow and AO prepasses do not run this
// function, so there only that engine cut applies.
// Colour from M_Particle_Dust_01.mat.

const DUST_COLOR: vec3<f32> = vec3<f32>(0.3456, 0.2736, 0.2329); // sRGB 0.623, 0.560, 0.520
const DUST_CLIP: f32 = 0.5;

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    let mask = texture_at(in, 0u, in.uv).r;
    if mask < DUST_CLIP {
        discard;
    }
    var o = out;
    o.albedo = DUST_COLOR;
    o.metallic = 0.0;
    o.smoothness = 0.5;
    return o;
}
