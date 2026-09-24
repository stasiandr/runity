//! Gizmos: the render module's, with the physics module's colliders drawn
//! by them — where the two meet, so neither knows the other.

pub use scrap_render::gizmo::*;

use crate::material::Material;
use crate::render::{Draw, MeshHandle};
use glam::Mat4;

/// The colour a collider's outline is drawn in, by what kind of body it
/// is: green stays, blue falls, orange is moved by the game, yellow is a
/// zone.
pub fn collider_color(body: crate::scene::Body) -> Material {
    use crate::scene::Body;
    let [r, g, b] = match body {
        Body::None => [0.6, 0.6, 0.6],
        Body::Static => [0.2, 0.85, 0.3],
        Body::Dynamic => [0.25, 0.5, 1.0],
        Body::Kinematic => [1.0, 0.55, 0.15],
        Body::Trigger => [1.0, 0.9, 0.2],
        // A part is drawn in its body's colour by whoever knows the body;
        // alone it is a solid shape: green, and a zone yellow.
        Body::Part => [0.35, 0.8, 0.55],
        Body::TriggerPart => [1.0, 0.8, 0.3],
    };
    Material::new(r, g, b).unlit()
}

/// A collider as lines ([`crate::scene::Collider::outline`]): the shape
/// physics sees, drawn over what the eye sees. `arm` is a unit cube, as
/// for the handles; `thickness` is a line's width in metres.
pub fn collider_draws(
    arm: MeshHandle,
    shape: crate::scene::Collider,
    placed: Mat4,
    thickness: f32,
    material: Material,
) -> Vec<Draw> {
    let (segments, frame) = shape.outline(placed);
    line_draws(arm, segments, frame, thickness, material)
}
