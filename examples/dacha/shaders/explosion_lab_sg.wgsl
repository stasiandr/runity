// From Assets/Thirdparty/VFX_Klaus/Shaders/Explosion_lab_SG.shadergraph (URP Unlit).
// The VFX demo room's backdrop: an unlit two-colour gradient by height,
// Down colour below, Up colour above, blended over a short band.
// Port: uses world y where the graph used object-space y. Unlit is
// imitated by black albedo and the colour as emission (fog still applies).
// Values from VFX_lab.mat.

const EXPLOSION_DOWN: vec3<f32> = vec3<f32>(0.0567, 0.0564, 0.0562); // sRGB 0.264, 0.263, 0.263
const EXPLOSION_UP: vec3<f32> = vec3<f32>(0.0116, 0.0125, 0.0221); // sRGB 0.110, 0.115, 0.160
const EXPLOSION_HEIGHT: f32 = 0.03;
const EXPLOSION_THICKNESS: f32 = 5.0;

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let t = clamp((in.world_position.y - EXPLOSION_HEIGHT) * EXPLOSION_THICKNESS, 0.0, 1.0);
    o.albedo = vec3<f32>(0.0);
    o.emission = mix(EXPLOSION_DOWN, EXPLOSION_UP, t);
    o.metallic = 0.0;
    o.smoothness = 0.0;
    return o;
}
