// From Assets/Thirdparty/VFX_Klaus/Shaders/Fx_dissolve_particle_apb.shadergraph (URP Unlit, transparent).
// A splash sprite that dissolves over its life: the texture's G channel,
// pushed by particle custom data (progress, edge sharpness, UV
// distortion by the B channel), eats the A-channel shape away; colour is
// vertex colour times R, lerped toward a custom-data highlight colour by a
// remapped band of the same dissolve value; depth-soft edges.
// Port: the sprite sheet (Main_tex: Tex_fx_splash_set / Tex_fx_elements_set_01)
// is read from the material. The particle's colour and fade reach the
// shader only already multiplied by the base map (the same sheet), so they
// are recovered by dividing the base map back out (a sheet channel that is
// black there loses its tint; the grey the channels share stands in). There
// are no custom data streams, so life is read from runity's fade as
// 1 - alpha, and custom data follows the curves in Booooom.prefab
// (progress = life, sharpness = 20 * life, distortion = -0.1 * life). The
// highlight colour is white; its band's range, Highlight_Min and
// Highlight_Max, comes from the material (-10..-2 in the inner ones, where
// it never shows; 10..2 in the outer). Soft particles are dropped (no scene
// depth). Unlit is imitated by black albedo and colour as emission.
// runity:params Vector1_930B327D Vector1_270105AC
// runity:textures Main_tex

const FXD_EMISSION_POWER: f32 = 1.0;
const FXD_HIGHLIGHT: vec4<f32> = vec4<f32>(1.0, 1.0, 1.0, 1.0); // custom data 2: rgb, strength
// Below this a base map channel is taken as black: its tint is lost there.
const FXD_SEEN: f32 = 0.02;

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    // The sheet unmoved, as the base map was read: what the particle's
    // colour and fade were multiplied by.
    let plain = texture_at(in, 0u, in.uv);
    let fade = clamp(out.alpha / max(plain.a, 1e-3), 0.0, 1.0);
    let shared_tint = (out.albedo.r + out.albedo.g + out.albedo.b) / max(plain.r + plain.g + plain.b, 1e-3);
    let tint = select(vec3<f32>(shared_tint), out.albedo / max(plain.rgb, vec3<f32>(1e-3)), plain.rgb > vec3<f32>(FXD_SEEN));

    let life = 1.0 - fade;
    let progress = life;
    let sharpness = 20.0 * life;
    let distortion = -0.1 * life;

    // Unity's UV's u, pushed along by the sheet's B channel.
    let warp = plain.b * distortion;
    let t = texture_at(in, 0u, vec2<f32>(in.uv.x + warp, in.uv.y));

    let dissolve = t.g + 1.0 - progress;
    let out_min = -(progress * sharpness);
    let mask = clamp(out_min + dissolve * (1.0 - out_min) / (1.0 + 0.1 * progress), 0.0, 1.0);

    let band = clamp(in.params[0].x + dissolve * (in.params[0].y - in.params[0].x), 0.0, 1.0);
    let color = mix(tint * t.r * mask, FXD_HIGHLIGHT.rgb * FXD_EMISSION_POWER, fade * FXD_HIGHLIGHT.a * band);

    o.albedo = vec3<f32>(0.0);
    o.emission = color;
    o.alpha = t.a * mask * fade;
    o.metallic = 0.0;
    o.smoothness = 0.0;
    return o;
}
