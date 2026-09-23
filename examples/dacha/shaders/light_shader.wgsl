// From Assets/Content/Art/Materials/Light_Shader.shadergraph (URP Sprite
// Unlit, alpha blended, both faces). Values from M_Light.mat (M_Light 1.mat
// has _Alpha 0.3 and _Color_Int 0.3).
//
// A fake light beam: _Color x _Color_Int, with alpha = streaks from
// T_SunLines_M x _Alpha, minus a second, stretched copy of the same streaks
// scrolling slowly upwards, so the beam shimmers.
//
// This port: T_SunLines_M is replaced by procedural soft streaks; the colour
// goes to emission over a black albedo. The .mat says _Surface: 0, so unless
// the material is made transparent the alpha has no effect.

const LIGHT_COLOR: vec3<f32> = vec3<f32>(1.0, 0.6444, 0.4126);
const LIGHT_COLOR_INT: f32 = 3.4;
const LIGHT_ALPHA: f32 = 0.6;
const LIGHT_TILING: vec2<f32> = vec2<f32>(0.67, 0.37);
const LIGHT_SPEED: f32 = 0.01;

fn light_hash(p: vec2<f32>) -> f32 {
    let q = vec2<u32>(vec2<i32>(floor(p)) + vec2<i32>(32768));
    var h = q.x * 1597334677u ^ q.y * 3812015801u;
    h = h * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h) / 4294967295.0;
}

// Stand-in for T_SunLines_M (Unity UV, repeating): soft horizontal streaks
// of uneven width and brightness, fading out towards the left and right.
fn light_sunlines(uv: vec2<f32>) -> f32 {
    let y = uv.y * 14.0;
    let row = floor(y);
    let f = fract(y);
    let width = mix(0.08, 0.35, light_hash(vec2<f32>(row, 7.0)));
    let d = (f - 0.5) / width;
    let bright = 0.6 * pow(light_hash(vec2<f32>(row, 3.0)), 1.5);
    let x = fract(uv.x);
    let centre = mix(0.35, 0.65, light_hash(vec2<f32>(row, 11.0)));
    let along = exp(-pow((x - centre) / 0.3, 2.0)) * 0.6 + 0.4;
    let edges = smoothstep(0.08, 0.3, x) * (1.0 - smoothstep(0.7, 0.9, x));
    return bright * exp(-d * d) * along * edges;
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    // Unity's v runs up the texture; runity's runs down.
    let uv = vec2<f32>(in.uv.x, 1.0 - in.uv.y);
    let beam = light_sunlines(uv) * LIGHT_ALPHA;
    let moving = light_sunlines(uv * LIGHT_TILING + vec2<f32>(0.0, LIGHT_SPEED * in.time));
    o.albedo = vec3<f32>(0.0);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.emission = LIGHT_COLOR * LIGHT_COLOR_INT;
    o.alpha = clamp(beam - moving, 0.0, 1.0);
    return o;
}
