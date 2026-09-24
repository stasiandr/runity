// Smoke on the GPU: the stable-fluids step of `runity_fluid::smoke`, a
// thread a cell. Each pass reads what the one before wrote; what needs its
// neighbours' old values writes into a second buffer, copied back after.

struct Params {
    // cells along x, y, z; cells in all
    n: vec4<u32>,
    // cell size (m), step (s), the step's time, source flicker
    step: vec4<f32>,
    // source radius (cells), smoke a second, heat a second, most heat
    source: vec4<f32>,
    // weight, curl strength, what is kept a step (fade), unused
    look: vec4<f32>,
    // the wind the air is drawn toward (m/s), unused
    wind: vec4<f32>,
    // the picture's cells along x, y, z, the slab's first z
    pack: vec4<u32>,
    // every so many cells, along x, y, z; heat that is white-hot
    stride: vec4<f32>,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> vel: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> vel_next: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> dens: array<f32>;
@group(0) @binding(4) var<storage, read_write> dens_next: array<f32>;
@group(0) @binding(5) var<storage, read_write> heat: array<f32>;
@group(0) @binding(6) var<storage, read_write> heat_next: array<f32>;
@group(0) @binding(7) var<storage, read_write> pres: array<f32>;
@group(0) @binding(8) var<storage, read_write> pres_next: array<f32>;
@group(0) @binding(9) var<storage, read_write> div: array<f32>;
@group(0) @binding(10) var<storage, read_write> curl: array<vec4<f32>>;
@group(0) @binding(11) var<storage, read> solid: array<u32>;
@group(0) @binding(12) var picture: texture_storage_3d<rgba8unorm, write>;

fn at(i: u32, j: u32, k: u32) -> u32 {
    return (k * p.n.y + j) * p.n.x + i;
}

fn inside(id: vec3<u32>) -> bool {
    return id.x < p.n.x && id.y < p.n.y && id.z < p.n.z;
}

fn interior(id: vec3<u32>) -> bool {
    return id.x > 0u && id.y > 0u && id.z > 0u && id.x + 1u < p.n.x && id.y + 1u < p.n.y && id.z + 1u < p.n.z;
}

// Between the eight cells round `q` (in cells, their middles at +0.5).
fn sample_vel(q: vec3<f32>) -> vec3<f32> {
    let c = clamp(q - vec3<f32>(0.5), vec3<f32>(0.0), vec3<f32>(f32(p.n.x) - 1.001, f32(p.n.y) - 1.001, f32(p.n.z) - 1.001));
    let b = vec3<u32>(c);
    let f = c - vec3<f32>(b);
    let x00 = mix(vel[at(b.x, b.y, b.z)].xyz, vel[at(b.x + 1u, b.y, b.z)].xyz, f.x);
    let x10 = mix(vel[at(b.x, b.y + 1u, b.z)].xyz, vel[at(b.x + 1u, b.y + 1u, b.z)].xyz, f.x);
    let x01 = mix(vel[at(b.x, b.y, b.z + 1u)].xyz, vel[at(b.x + 1u, b.y, b.z + 1u)].xyz, f.x);
    let x11 = mix(vel[at(b.x, b.y + 1u, b.z + 1u)].xyz, vel[at(b.x + 1u, b.y + 1u, b.z + 1u)].xyz, f.x);
    return mix(mix(x00, x10, f.y), mix(x01, x11, f.y), f.z);
}

fn sample_scalar(q: vec3<f32>, which: u32) -> f32 {
    let c = clamp(q - vec3<f32>(0.5), vec3<f32>(0.0), vec3<f32>(f32(p.n.x) - 1.001, f32(p.n.y) - 1.001, f32(p.n.z) - 1.001));
    let b = vec3<u32>(c);
    let f = c - vec3<f32>(b);
    var s: array<f32, 8>;
    for (var n = 0u; n < 8u; n = n + 1u) {
        let a = at(b.x + (n & 1u), b.y + ((n >> 1u) & 1u), b.z + (n >> 2u));
        s[n] = select(heat[a], dens[a], which == 0u);
    }
    let x00 = mix(s[0], s[1], f.x);
    let x10 = mix(s[2], s[3], f.x);
    let x01 = mix(s[4], s[5], f.x);
    let x11 = mix(s[6], s[7], f.x);
    return mix(mix(x00, x10, f.y), mix(x01, x11, f.y), f.z);
}

// The source's smoke and heat, then the forces on the air.
@compute @workgroup_size(4, 4, 4)
fn cs_forces(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) {
        return;
    }
    let a = at(id.x, id.y, id.z);
    let dt = p.step.y;
    let r = p.source.x;
    let rows = u32(clamp(r, 1.0, 3.0));
    if id.y >= 1u && id.y <= rows {
        let d = length(vec2<f32>(f32(id.x) + 0.5 - f32(p.n.x) * 0.5, f32(id.z) + 0.5 - f32(p.n.z) * 0.5));
        if d < r {
            let fall = 1.0 - d / r;
            let flicker = p.step.w;
            dens[a] = min(dens[a] + p.source.y * dt * fall * flicker, 3.0);
            heat[a] = min(heat[a] + p.source.z * dt * fall * flicker, max(p.source.w, 0.1));
        }
    }
    var v = vel[a].xyz;
    let lift = heat[a] * 3.0 - dens[a] * p.look.x;
    v.y = v.y + lift * dt;
    let toward = p.wind.xyz - v;
    v = v + toward * min(1.5 * dt * (0.3 + f32(id.y) / f32(p.n.y)), 1.0);
    vel[a] = vec4<f32>(v, 0.0);
}

@compute @workgroup_size(4, 4, 4)
fn cs_curl(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) {
        return;
    }
    let a = at(id.x, id.y, id.z);
    if !interior(id) {
        curl[a] = vec4<f32>(0.0);
        return;
    }
    let nx = p.n.x;
    let slice = p.n.x * p.n.y;
    let dvz_dy = vel[a + nx].z - vel[a - nx].z;
    let dvy_dz = vel[a + slice].y - vel[a - slice].y;
    let dvx_dz = vel[a + slice].x - vel[a - slice].x;
    let dvz_dx = vel[a + 1u].z - vel[a - 1u].z;
    let dvy_dx = vel[a + 1u].y - vel[a - 1u].y;
    let dvx_dy = vel[a + nx].x - vel[a - nx].x;
    curl[a] = vec4<f32>(vec3<f32>(dvz_dy - dvy_dz, dvx_dz - dvz_dx, dvy_dx - dvx_dy) * (0.5 / p.step.x), 0.0);
}

@compute @workgroup_size(4, 4, 4)
fn cs_confine(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) || !interior(id) {
        return;
    }
    let a = at(id.x, id.y, id.z);
    let nx = p.n.x;
    let slice = p.n.x * p.n.y;
    let grad = vec3<f32>(
        length(curl[a + 1u].xyz) - length(curl[a - 1u].xyz),
        length(curl[a + nx].xyz) - length(curl[a - nx].xyz),
        length(curl[a + slice].xyz) - length(curl[a - slice].xyz),
    );
    var n = vec3<f32>(0.0);
    if dot(grad, grad) > 0.0 {
        n = normalize(grad);
    }
    vel[a] = vec4<f32>(vel[a].xyz + cross(n, curl[a].xyz) * (p.look.y * p.step.x * p.step.y), 0.0);
}

// The speed carried along itself, and none into what is solid or the floor.
@compute @workgroup_size(4, 4, 4)
fn cs_advect_vel(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) {
        return;
    }
    let a = at(id.x, id.y, id.z);
    let q = vec3<f32>(id) + 0.5 - vel[a].xyz * p.step.y / p.step.x;
    var v = sample_vel(q);
    if solid[a] != 0u || id.y == 0u {
        v = vec3<f32>(0.0);
    }
    vel_next[a] = vec4<f32>(v, 0.0);
}

@compute @workgroup_size(4, 4, 4)
fn cs_divergence(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) {
        return;
    }
    let a = at(id.x, id.y, id.z);
    if !interior(id) {
        div[a] = 0.0;
        return;
    }
    let nx = p.n.x;
    let slice = p.n.x * p.n.y;
    div[a] = (vel[a + 1u].x - vel[a - 1u].x + vel[a + nx].y - vel[a - nx].y + vel[a + slice].z - vel[a - slice].z) * 0.5;
}

fn pressure_at(a: u32, here: u32) -> f32 {
    return select(pres[a], pres[here], solid[a] != 0u);
}

@compute @workgroup_size(4, 4, 4)
fn cs_jacobi(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) {
        return;
    }
    let a = at(id.x, id.y, id.z);
    if !interior(id) || solid[a] != 0u {
        pres_next[a] = pres[a];
        return;
    }
    let nx = p.n.x;
    let slice = p.n.x * p.n.y;
    pres_next[a] = (pressure_at(a - 1u, a) + pressure_at(a + 1u, a) + pressure_at(a - nx, a) + pressure_at(a + nx, a)
        + pressure_at(a - slice, a) + pressure_at(a + slice, a) - div[a]) / 6.0;
}

// The sweep back: the next pressure's neighbours into this one, so two
// sweeps need no copy.
fn pressure_next_at(a: u32, here: u32) -> f32 {
    return select(pres_next[a], pres_next[here], solid[a] != 0u);
}

@compute @workgroup_size(4, 4, 4)
fn cs_jacobi_back(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) {
        return;
    }
    let a = at(id.x, id.y, id.z);
    if !interior(id) || solid[a] != 0u {
        pres[a] = pres_next[a];
        return;
    }
    let nx = p.n.x;
    let slice = p.n.x * p.n.y;
    pres[a] = (pressure_next_at(a - 1u, a) + pressure_next_at(a + 1u, a) + pressure_next_at(a - nx, a) + pressure_next_at(a + nx, a)
        + pressure_next_at(a - slice, a) + pressure_next_at(a + slice, a) - div[a]) / 6.0;
}

// What was made into the second buffers, back into the first.
@compute @workgroup_size(4, 4, 4)
fn cs_keep_vel(@builtin(global_invocation_id) id: vec3<u32>) {
    if inside(id) {
        let a = at(id.x, id.y, id.z);
        vel[a] = vel_next[a];
    }
}

@compute @workgroup_size(4, 4, 4)
fn cs_keep_scalars(@builtin(global_invocation_id) id: vec3<u32>) {
    if inside(id) {
        let a = at(id.x, id.y, id.z);
        dens[a] = dens_next[a];
        heat[a] = heat_next[a];
    }
}

@compute @workgroup_size(4, 4, 4)
fn cs_gradient(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) || !interior(id) {
        return;
    }
    let a = at(id.x, id.y, id.z);
    let nx = p.n.x;
    let slice = p.n.x * p.n.y;
    let g = vec3<f32>(pres[a + 1u] - pres[a - 1u], pres[a + nx] - pres[a - nx], pres[a + slice] - pres[a - slice]) * 0.5;
    vel[a] = vec4<f32>(vel[a].xyz - g, 0.0);
}

// Smoke and heat carried along the speed, fading.
@compute @workgroup_size(4, 4, 4)
fn cs_advect_scalars(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) {
        return;
    }
    let a = at(id.x, id.y, id.z);
    if solid[a] != 0u {
        dens_next[a] = 0.0;
        heat_next[a] = 0.0;
        return;
    }
    let q = vec3<f32>(id) + 0.5 - vel[a].xyz * p.step.y / p.step.x;
    dens_next[a] = sample_scalar(q, 0u) * p.look.z;
    heat_next[a] = sample_scalar(q, 1u) * p.look.z;
}

// Into the fog's picture: every so many cells, a byte of density (0–3)
// and one of heat, in the smoke's slab.
@compute @workgroup_size(4, 4, 4)
fn cs_pack(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.pack.x || id.y >= p.pack.y || id.z >= p.pack.z {
        return;
    }
    let s = vec3<u32>(p.stride.xyz);
    let a = at(id.x * s.x, id.y * s.y, id.z * s.z);
    let d = clamp(dens[a] / 3.0, 0.0, 1.0);
    let h = clamp(heat[a] / p.stride.w, 0.0, 1.0);
    textureStore(picture, vec3<u32>(id.x, id.y, id.z + p.pack.w), vec4<f32>(d, h, 0.0, 1.0));
}
