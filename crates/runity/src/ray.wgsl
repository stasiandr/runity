// The traced half of the lit shader (ray.rs): what a point sees, asked of
// the scene itself through the hardware's ray queries.

@group(0) @binding(5) var scene_rays: acceleration_structure;

/// 1.0 when nothing among `mask` (RAY_THINGS, RAY_TERRAIN) lies between
/// `start` and `reach` along `direction` from `origin`, 0.0 when something
/// does. Everything in the scene is opaque to rays.
fn ray_clear(origin: vec3<f32>, direction: vec3<f32>, start: f32, reach: f32, mask: u32) -> f32 {
    var query: ray_query;
    rayQueryInitialize(
        &query,
        scene_rays,
        RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT, mask, start, reach, origin, direction),
    );
    rayQueryProceed(&query);
    let hit = rayQueryGetCommittedIntersection(&query);
    return select(1.0, 0.0, hit.kind != RAY_QUERY_INTERSECTION_NONE);
}
