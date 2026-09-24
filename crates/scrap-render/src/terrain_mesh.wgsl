// Terrain's fine grid by mesh shaders (terrain.rs): the same rings as the
// vertex-shader grid, cut into patches of 8×8 cells. A task shader keeps
// the patches worth drawing — in view, on the terrain, not in the hole
// the ring inside fills — and a mesh shader makes each one's 81 vertices
// and 128 triangles with the same `terrain_vertex`, so the two ways of
// drawing it are one surface, crack for crack. Appended to render.wgsl,
// after `enable wgpu_mesh_shader;`, only where the device has them.
//
// The task and mesh stages read nothing of the frame's group: in wgpu 30 a
// group seen by them and by the fragment stage is laid out one way and
// bound another, and the fragment stage gets the wrong resources. They
// have a group of their own (3) — what of the frame they need, and the
// heights — and their own copies of the terrain functions, made from
// render.wgsl's by the renderer (`tframe.` read as `tframe.`, names `_m`).

struct TerrainWind {
    wind: vec4<f32>,
};

struct TerrainFrame {
    view_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    foliage: TerrainWind,
    terrain_to_local: mat4x4<f32>,
    terrain_to_world: mat4x4<f32>,
    terrain: vec4<f32>,
    terrain_bounds: vec4<f32>,
    terrain_look: array<vec4<f32>, 7>,
};

@group(3) @binding(0) var<uniform> tframe: TerrainFrame;
@group(3) @binding(1) var mesh_heights: texture_2d<f32>;

// A patch is 8 by 8 cells: 81 vertices, 128 triangles. Bigger ones hold
// more of what they make in the workgroup's memory, and fewer run at once
// — slower on the whole (measured: 16 by 8 was a quarter slower).
const PATCH_X: i32 = 8;
const PATCH_Z: i32 = 8;
const PATCHES_X: i32 = 18;
const PATCHES_Z: i32 = 18;
const RINGS: u32 = 9u;

// Each task invocation looks at this many patches: a task workgroup costs
// the GPU the same whatever it keeps, so few of them, each doing more.
const PER_THREAD: u32 = 8u;

struct Patches {
    ids: array<u32, 512>,
};

var<task_payload> kept_patches: Patches;
var<workgroup> kept: atomic<u32>;

// Which ring a patch is in, and its first cell there.
fn patch_of(index: u32) -> vec3<i32> {
    let per_ring = u32(PATCHES_X * PATCHES_Z);
    let level = i32(index / per_ring);
    let rest = i32(index % per_ring);
    let px = rest % PATCHES_X - PATCHES_X / 2;
    let pz = rest / PATCHES_X - PATCHES_Z / 2;
    return vec3<i32>(level, px * PATCH_X, pz * PATCH_Z);
}

// Whether a patch is drawn: not in the hole the ring inside fills, on the
// terrain, and — its box from the ground's lowest to highest — in view.
fn patch_kept(index: u32) -> bool {
    if index >= RINGS * u32(PATCHES_X * PATCHES_Z) || tframe.terrain.z < 0.5 {
        return false;
    }
    let p = patch_of(index);
    if p.x > 0 && p.y >= -32 && p.y < 32 && p.z >= -32 && p.z < 32 {
        return false;
    }
    let spacing = tframe.terrain.w * exp2(f32(p.x));
    let snap = spacing * 2.0;
    let centre = floor(tframe.camera_position.xz / snap) * snap;
    let lo = centre + vec2<f32>(f32(p.y), f32(p.z)) * spacing;
    let hi = lo + vec2<f32>(f32(PATCH_X), f32(PATCH_Z)) * spacing;
    // Off the terrain on one side altogether.
    let half = tframe.terrain.x * 0.5;
    var off = vec4<bool>(true);
    for (var k = 0; k < 4; k = k + 1) {
        let c = vec2<f32>(select(lo.x, hi.x, (k & 1) != 0), select(lo.y, hi.y, (k & 2) != 0));
        let l = (tframe.terrain_to_local * vec4<f32>(c.x, 0.0, c.y, 1.0)).xz;
        off = off & vec4<bool>((l.x < -half), (l.x > half), (l.y < -half), (l.y > half));
    }
    if any(off) {
        return false;
    }
    // Out of view: the box wholly behind one of the frustum's planes, each
    // tested at the corner furthest along its normal.
    // Its own height: the ground at nine points of it, give or take what
    // the slope between them and the ripples could add.
    var bottom = 1.0e9;
    var top = -1.0e9;
    for (var k = 0; k < 9; k = k + 1) {
        let at = mix(lo, hi, vec2<f32>(f32(k % 3), f32(k / 3)) * 0.5);
        let h = terrain_ground_m(at, 0.0, 0.0, false);
        bottom = min(bottom, h);
        top = max(top, h);
    }
    let slack = (top - bottom) * 0.5 + max(hi.x - lo.x, hi.y - lo.y) * 0.25 + 0.1;
    let box_lo = vec3<f32>(lo.x, max(bottom - slack, tframe.terrain_bounds.x), lo.y);
    let box_hi = vec3<f32>(hi.x, min(top + slack, tframe.terrain_bounds.y), hi.y);
    let m = tframe.view_projection;
    let r0 = vec4<f32>(m[0].x, m[1].x, m[2].x, m[3].x);
    let r1 = vec4<f32>(m[0].y, m[1].y, m[2].y, m[3].y);
    let r2 = vec4<f32>(m[0].z, m[1].z, m[2].z, m[3].z);
    let r3 = vec4<f32>(m[0].w, m[1].w, m[2].w, m[3].w);
    let planes = array<vec4<f32>, 5>(r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2);
    for (var k = 0; k < 5; k = k + 1) {
        let plane = planes[k];
        let far = select(box_lo, box_hi, plane.xyz > vec3<f32>(0.0));
        if dot(plane.xyz, far) + plane.w < 0.0 {
            return false;
        }
    }
    return true;
}

@task
@payload(kept_patches)
@workgroup_size(64)
fn ts_terrain(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) li: u32) -> @builtin(mesh_task_size) vec3<u32> {
    if li == 0u {
        atomicStore(&kept, 0u);
    }
    workgroupBarrier();
    for (var k = 0u; k < PER_THREAD; k = k + 1u) {
        let index = id.x * PER_THREAD + k;
        if patch_kept(index) {
            let slot = atomicAdd(&kept, 1u);
            kept_patches.ids[slot] = index;
        }
    }
    workgroupBarrier();
    return vec3<u32>(atomicLoad(&kept), 1u, 1u);
}

struct TerrainPrimitive {
    @builtin(triangle_indices) indices: vec3<u32>,
};

struct TerrainMesh {
    @builtin(vertices) vertices: array<VertexOutput, 81>,
    @builtin(primitives) primitives: array<TerrainPrimitive, 128>,
    @builtin(vertex_count) vertex_count: u32,
    @builtin(primitive_count) primitive_count: u32,
};

var<workgroup> terrain_mesh: TerrainMesh;

// Heights of the patch's grid with a border of two cells: each worked
// out once and shared by the workgroup — the mesh shader's gain over the
// vertex shader, which must work out five for every vertex it makes.
var<workgroup> ground: array<f32, 169>;

// Where cell `g` of ring `level` lies, before any folding.
fn cell_xz(g: vec2<f32>, spacing: f32) -> vec2<f32> {
    let snap = spacing * 2.0;
    let centre = floor(tframe.camera_position.xz / snap) * snap;
    return centre + g * spacing;
}

// How far toward the next ring's grid a cell is folded (0 to 1).
fn fold_of(g: vec2<f32>, spacing: f32) -> f32 {
    let first = cell_xz(g, spacing);
    let eye = tframe.camera_position.xz;
    let d = max(abs(first.x - eye.x), abs(first.y - eye.y)) / spacing;
    return clamp((d - 44.0) / 18.0, 0.0, 1.0);
}

// 13 by 13: the patch's 9 by 9 points and two round them.
fn ground_at(i: i32, j: i32) -> f32 {
    return ground[u32((j + 2) * 13 + (i + 2))];
}

@mesh(terrain_mesh)
@payload(kept_patches)
@workgroup_size(64)
fn ms_terrain(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let p = patch_of(kept_patches.ids[group.x]);
    let origin = vec2<f32>(f32(p.y), f32(p.z));
    let level = f32(p.x);
    let spacing = tframe.terrain.w * exp2(level);
    let look = TerrainLook(
        tframe.terrain_look[0],
        tframe.terrain_look[1],
        tframe.terrain_look[2],
        tframe.terrain_look[3],
        tframe.terrain_look[4],
        tframe.terrain_look[5],
        tframe.terrain_look[6],
    );
    let sand = look.color_and_shading.w > 3.5;
    // The ground at every point of the grid and its border, once.
    for (var k = li; k < 169u; k = k + 64u) {
        let g = origin + vec2<f32>(f32(i32(k % 13u) - 2), f32(i32(k / 13u) - 2));
        let span = spacing * (1.0 + fold_of(g, spacing));
        let fine = 1.0 - smoothstep(0.02, 0.045, span);
        let coarse = 1.0 - smoothstep(0.1, 0.2, span);
        ground[k] = terrain_ground_m(cell_xz(g, spacing), fine, coarse, sand);
    }
    workgroupBarrier();
    if li == 0u {
        terrain_mesh.vertex_count = 81u;
        terrain_mesh.primitive_count = 128u;
    }
    for (var v = li; v < 81u; v = v + 64u) {
        let i = i32(v % 9u);
        let j = i32(v / 9u);
        let g = origin + vec2<f32>(f32(i), f32(j));
        let morph = fold_of(g, spacing);
        // Odd cells slide toward the even one before them as they fold:
        // their height and slope slide with them, between the two.
        let odd = fract(g * 0.5) * 2.0;
        let t = odd * morph;
        let xz = cell_xz(g - t, spacing);
        let h0 = ground_at(i, j);
        let hx = ground_at(i - 1, j);
        let hz = ground_at(i, j - 1);
        let hxz = ground_at(i - 1, j - 1);
        let y = mix(mix(h0, hx, t.x), mix(hz, hxz, t.x), t.y);
        // Slopes from the neighbours, at the point and the one it folds to.
        let sx0 = (ground_at(i + 1, j) - ground_at(i - 1, j)) / (2.0 * spacing);
        let sz0 = (ground_at(i, j + 1) - ground_at(i, j - 1)) / (2.0 * spacing);
        let sx1 = (ground_at(i, j) - ground_at(i - 2, j)) / (2.0 * spacing);
        let sz1 = (ground_at(i, j) - ground_at(i, j - 2)) / (2.0 * spacing);
        let sx = mix(sx0, sx1, t.x);
        let sz = mix(sz0, sz1, t.y);
        let normal = normalize(vec3<f32>(-sx, 1.0, -sz));
        let span = spacing * (1.0 + morph);
        let fine = 1.0 - smoothstep(0.02, 0.045, span);
        let coarse = 1.0 - smoothstep(0.1, 0.2, span);
        let local = (tframe.terrain_to_local * vec4<f32>(xz.x, 0.0, xz.y, 1.0)).xz;
        let outside = any(abs(local) > vec2<f32>(tframe.terrain.x * 0.5));
        var out: VertexOutput;
        let world = vec3<f32>(xz.x, y, xz.y);
        out.clip_position = tframe.view_projection * vec4<f32>(world, 1.0);
        out.world_position = world;
        out.normal = normal;
        out.base_color = look.color_and_shading.rgb;
        out.shading = look.color_and_shading.w;
        out.uv = xz * look.uv_transform.xy + look.uv_transform.zw;
        out.surface = vec4<f32>(look.surface.xyz, select(0.0, 2.0, outside));
        out.emission = look.emission;
        out.detail = look.detail;
        out.params_0 = look.params_0;
        out.params_1 = vec4<f32>(look.params_1.xy, fine, coarse);
        out.subsurface = vec4<f32>(0.0, 0.0, 0.0, 0.01);
        out.maps = vec4<u32>(0u);
        terrain_mesh.vertices[v] = out;
    }
    // Counter-clockwise seen from above, as the vertex-shader grid is.
    let i = li % 8u;
    let j = li / 8u;
    let a = j * 9u + i;
    terrain_mesh.primitives[2u * li].indices = vec3<u32>(a, a + 9u, a + 1u);
    terrain_mesh.primitives[2u * li + 1u].indices = vec3<u32>(a + 1u, a + 9u, a + 10u);
}
