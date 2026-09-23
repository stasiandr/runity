// From Assets/Content/Containers/Water.shader ("Dacha/Water"), values from
// M_GreyboxWater.mat.
//
// The original is a small unlit transparent surface for water in vessels: a
// tint that darkens towards grazing angles, a bright rim, and one Blinn-Phong
// sun highlight on a normal tilted by a few world-space sines.
//
// This port does the same maths and puts the result in `emission` with a black
// albedo, so the engine's own lighting adds almost nothing (the original is
// unlit). The highlight uses the engine's sun direction and colour, which may
// be scaled differently from URP's main light. The material's .mat says
// _Surface 0, so the importer may bring it in as opaque and ignore `alpha`.

const WATER_SHALLOW = vec4<f32>(0.045, 0.2, 0.34, 0.42);
const WATER_DEEP = vec4<f32>(0.01, 0.06, 0.15, 0.8);
const WATER_RIM_COLOR = vec3<f32>(0.55, 0.78, 0.95);
const WATER_RIM_POWER = 2.5;
const WATER_RIM_STRENGTH = 0.35;
const WATER_GLOSS = 48.0;
const WATER_SPEC_STRENGTH = 0.7;
const WATER_RIPPLE_SCALE = 34.0;
const WATER_RIPPLE_SPEED = 1.5;
const WATER_RIPPLE_STRENGTH = 0.14;

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let flat_normal = normalize(in.normal);

    let p = in.world_position.xz * WATER_RIPPLE_SCALE;
    let t = in.time * WATER_RIPPLE_SPEED;
    let slope = vec2<f32>(
        cos(p.x + t) * 0.6 + cos(p.x * 1.7 - t * 0.7 + p.y * 0.4) * 0.4,
        sin(p.y * 1.3 - t * 0.8) * 0.6 + sin(p.y * 2.1 + t * 0.5 + p.x * 0.3) * 0.4,
    );
    let n = normalize(flat_normal + vec3<f32>(slope.x, 0.0, slope.y) * WATER_RIPPLE_STRENGTH);

    let view = normalize(frame.camera_position.xyz - in.world_position);
    let facing = saturate(dot(flat_normal, view));
    let body = mix(WATER_DEEP, WATER_SHALLOW, facing);
    let rim = pow(1.0 - facing, WATER_RIM_POWER) * WATER_RIM_STRENGTH;

    let to_sun = -normalize(frame.sun_direction.xyz);
    let half_dir = normalize(to_sun + view);
    let spec = pow(saturate(dot(n, half_dir)), WATER_GLOSS) * WATER_SPEC_STRENGTH;

    o.albedo = vec3<f32>(0.0);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    o.normal = n;
    o.emission = body.rgb + WATER_RIM_COLOR * rim + frame.sun_color.rgb * spec;
    o.alpha = saturate(body.a + rim + spec * 0.25);
    return o;
}
