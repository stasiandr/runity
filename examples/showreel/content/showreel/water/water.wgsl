// Water: the pond's surface. A material with `shader: "water"` draws with
// this `surface` over the standard shader — rings running out across it,
// bending the light, darker where it is deep in the trough.

fn ripple(p: vec2<f32>, t: f32) -> f32 {
    return sin(length(p) * 6.0 - t * 2.5) * 0.5 + sin(p.x * 3.1 + p.y * 1.7 + t * 1.3) * 0.25;
}

fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    let p = in.world_position.xz;
    let e = 0.05;
    let h = ripple(p, in.time);
    let dx = ripple(p + vec2<f32>(e, 0.0), in.time) - h;
    let dz = ripple(p + vec2<f32>(0.0, e), in.time) - h;
    o.normal = normalize(in.normal + vec3<f32>(-dx, 0.0, -dz) * 1.5);
    o.albedo = mix(vec3<f32>(0.02, 0.10, 0.14), vec3<f32>(0.06, 0.22, 0.26), h * 0.5 + 0.5);
    o.smoothness = 0.92;
    return o;
}
