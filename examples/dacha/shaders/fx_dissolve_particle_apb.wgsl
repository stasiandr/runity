// From Assets/Thirdparty/VFX_Klaus/Shaders/Fx_dissolve_particle_apb.shadergraph (URP Unlit, transparent).
// A particle from a packed sheet (Main_tex: R the shape's shading, G the
// dissolve noise, B a distortion, A the shape) that dissolves over its
// life, lit at the dissolving edge by a glow colour.
// As the graph has it, with custom data 1 (progress p, sharpness s,
// distortion d) and custom data 2 (the highlight colour, HDR) from the
// particle — the emitter's streams, in this material's numbers:
//   u'     = u + B(uv) * d                      (the sheet read at u', v)
//   mask   = saturate(remap(G + 1 - p, [0, 1 + 0.1 p] -> [-p s, 1]))
//   band   = saturate(remap(G + 1 - p, [0, 1] -> [Highlight_Min, Highlight_Max]))
//   colour = lerp(vertex.rgb * R * mask, Emission_Power * c2.rgb, vertex.a * c2.a * band)
//   alpha  = A * mask * vertex.a
// The particle's colour and fade are the surface's albedo and alpha (no
// base map). Soft particles are dropped (no scene depth). Unlit: black
// albedo and the colour as emission.
// runity:params custom0.x custom0.y custom0.z custom1.r custom1.g custom1.b Vector1_930B327D Vector1_270105AC
// runity:textures Main_tex
// runity:base_map none

const FXD_EMISSION_POWER: f32 = 1.0;

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let progress = in.params[0].x;
    let sharpness = in.params[0].y;
    let distortion = in.params[0].z;
    let highlight = vec3<f32>(in.params[0].w, in.params[1].x, in.params[1].y);
    let highlight_min = in.params[1].z;
    let highlight_max = in.params[1].w;

    let warp = texture_at(in, 0u, in.uv).b * distortion;
    let t = texture_at(in, 0u, vec2<f32>(in.uv.x + warp, in.uv.y));

    let dissolve = t.g + 1.0 - progress;
    let out_min = -(progress * sharpness);
    let mask = clamp(out_min + dissolve * (1.0 - out_min) / (1.0 + 0.1 * progress), 0.0, 1.0);
    let band = clamp(highlight_min + dissolve * (highlight_max - highlight_min), 0.0, 1.0);

    let vertex = out.albedo;
    let fade = out.alpha;
    let color = mix(vertex * t.r * mask, highlight * FXD_EMISSION_POWER, fade * band);

    o.albedo = vec3<f32>(0.0);
    o.emission = color;
    o.alpha = t.a * mask * fade;
    o.metallic = 0.0;
    o.smoothness = 0.0;
    return o;
}
