// From Assets/Content/Buildings/Well/Well1/Animation/WaterPool.shadergraph,
// values from M_WaterPool.mat.
//
// The original is an unlit screen-depth water: an animated Voronoi field
// displaces the vertices and bends the screen UV; the scene colour behind is
// screen-blended with a surface-to-deep colour by scene depth, a reflection
// probe is pin-lit in by a Fresnel term, then shore foam, depth-banded foam
// particles, Voronoi highlights in _AdditionalColor and an interactive ripple
// render texture are lightened on top.
//
// Kept: the animated Voronoi (same hash as Unity's node), its Voronoi
// highlights, and its height as a normal. Dropped: vertex displacement,
// refraction and the scene colour (treated as black), shore foam and foam
// particles (both need scene depth), and the ripple render texture (a runtime
// texture; its ripple strength is 0 anyway). Scene depth for the colour is
// faked as a fixed water depth seen along the view. The reflection is left to
// the engine's own (Schlick, i.e. Fresnel power 5) with a high smoothness.
// Unlit is imitated with a black albedo and the colour in `emission`.

const WATER_POOL_SURFACE_COLOR = vec3<f32>(0.00275, 0.02695, 0.10561); // sRGB 0.036, 0.179, 0.358
const WATER_POOL_DEEP_COLOR = vec3<f32>(0.0, 0.16617, 1.0); // sRGB 0, 0.444, 1
const WATER_POOL_ADDITIONAL_COLOR = vec3<f32>(0.07973, 0.30899, 0.8563); // sRGB 0.313, 0.592, 0.934
const WATER_POOL_ADDITIONAL_POWER = 3.45;
const WATER_POOL_ADDITIONAL_MAX = 1.0;
const WATER_POOL_DEPTH_DISTANCE = 5.05;
const WATER_POOL_NOISE_SCALE = 7.39;
const WATER_POOL_TIME_NOISE = 2.0;
const WATER_POOL_NORMAL_STRENGTH = 0.01;
// Stand-in for the scene depth under the surface, in metres.
const WATER_POOL_ASSUMED_DEPTH = 1.0;
const WATER_POOL_SMOOTHNESS = 0.95;

fn water_pool_random_vector(uv: vec2<f32>, offset: f32) -> vec2<f32> {
    let r = fract(sin(vec2<f32>(uv.x * 15.27 + uv.y * 99.41, uv.x * 47.63 + uv.y * 89.98)) * 46839.32);
    return vec2<f32>(sin(r.y * offset) * 0.5 + 0.5, cos(r.x * offset) * 0.5 + 0.5);
}

// Unity's Voronoi node: distance to the nearest moving cell point.
fn water_pool_voronoi(uv: vec2<f32>, angle_offset: f32, cell_density: f32) -> f32 {
    let p = uv * cell_density;
    let g = floor(p);
    let f = fract(p);
    var best = 8.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let lattice = vec2<f32>(f32(x), f32(y));
            let offset = water_pool_random_vector(lattice + g, angle_offset);
            best = min(best, distance(lattice + offset, f));
        }
    }
    return best;
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let v = water_pool_voronoi(in.uv, in.time * WATER_POOL_TIME_NOISE, WATER_POOL_NOISE_SCALE);

    // Normal From Height, as Unity's node does it, from screen derivatives.
    let geometric = normalize(in.normal);
    let dpx = dpdx(in.world_position);
    let dpy = dpdy(in.world_position);
    let cross_x = cross(geometric, dpx);
    let cross_y = cross(dpy, geometric);
    let d = dot(dpx, cross_y);
    let scale = select(1.0, -1.0, d < 0.0) / max(abs(d), 1e-6);
    let grad = scale * (dpdx(v) * cross_y + dpdy(v) * cross_x);
    o.normal = normalize(geometric - WATER_POOL_NORMAL_STRENGTH * grad);

    let view = normalize(frame.camera_position.xyz - in.world_position);
    let facing = max(dot(geometric, view), 0.05);
    let depth_fade = saturate(WATER_POOL_ASSUMED_DEPTH / facing / WATER_POOL_DEPTH_DISTANCE);
    var color = mix(WATER_POOL_SURFACE_COLOR, WATER_POOL_DEEP_COLOR, depth_fade);

    // Lighten with _AdditionalColor where the Voronoi distance is large.
    let highlight = clamp(pow(v, WATER_POOL_ADDITIONAL_POWER), 0.0, WATER_POOL_ADDITIONAL_MAX);
    color = mix(color, max(color, WATER_POOL_ADDITIONAL_COLOR), highlight);

    o.albedo = vec3<f32>(0.0);
    o.metallic = 0.0;
    o.smoothness = WATER_POOL_SMOOTHNESS;
    o.emission = color;
    o.alpha = 1.0;
    return o;
}
