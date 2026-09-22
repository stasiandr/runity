//! The handles you drag to move something.
//!
//! Three parts, and the third is the one that is easy to get wrong:
//!
//! * a draw list, so the handles appear;
//! * a hit test, so a click picks one;
//! * a drag solver, which turns mouse movement into motion along an axis.
//!
//! The draws belong in [`crate::render::Frame::overlay_draws`], which is
//! drawn with the depth test off. A gizmo half inside the object it sits on
//! cannot be grabbed by anyone who cannot see it, and burying it is exactly
//! what depth testing does.
//!
//! The solver does not follow the cursor. It finds the point on the axis
//! closest to the ray under the cursor, and moves the object so that the
//! point first grabbed stays under the cursor. Following the cursor directly
//! makes the object jump to the pointer the instant it is grabbed, and makes
//! a near-edge-on axis fly off — which is the whole reason this is written
//! down rather than done by feel.

use glam::{Mat4, Vec3};

use crate::material::{Material, Shading};
use crate::render::{Camera, Draw, MeshHandle, TextureHandle};

/// Which handle is being pointed at or held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    X,
    Y,
    Z,
}

impl Handle {
    pub fn axis(self) -> Vec3 {
        match self {
            Handle::X => Vec3::X,
            Handle::Y => Vec3::Y,
            Handle::Z => Vec3::Z,
        }
    }

    /// Red, green, blue — the convention every tool shares, and one worth
    /// not being clever about.
    pub fn color(self) -> Material {
        let color = match self {
            Handle::X => Material::new(0.85, 0.16, 0.16),
            Handle::Y => Material::new(0.16, 0.75, 0.16),
            Handle::Z => Material::new(0.16, 0.35, 0.9),
        };
        Material {
            // Unlit, so a handle is the same colour whatever the sun is
            // doing and never disappears into a shadow.
            shading: Shading::Unlit,
            ..color
        }
    }

    pub const ALL: [Handle; 3] = [Handle::X, Handle::Y, Handle::Z];
}

/// How big the handles are and how forgiving they are to click.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GizmoStyle {
    /// Length of an arm, as a fraction of its distance from the camera.
    ///
    /// Scaled with distance rather than fixed in world units, so a gizmo is
    /// the same size on screen whether it is on a pebble or a mountain.
    pub screen_size: f32,
    /// Thickness of an arm, as a fraction of its length.
    pub thickness: f32,
    /// How close a ray has to pass to count as a hit, as a fraction of the
    /// arm's length. Generous on purpose: a handle that needs a pixel-exact
    /// click is a handle people stop using.
    pub grab_slack: f32,
}

impl Default for GizmoStyle {
    fn default() -> Self {
        Self {
            screen_size: 0.15,
            thickness: 0.04,
            grab_slack: 0.18,
        }
    }
}

impl GizmoStyle {
    /// The arm length for a gizmo at this position, seen from this camera.
    pub fn arm_length(&self, camera: &Camera, origin: Vec3) -> f32 {
        let distance = (origin - camera.position).length().max(0.01);
        distance * self.screen_size
    }
}

/// A grab in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drag {
    pub handle: Handle,
    /// Where the gizmo was when the grab started.
    pub origin: Vec3,
    /// How far along the axis the grab point was, so the object does not
    /// jump to the cursor on the first frame.
    pub grab_offset: f32,
}

/// Build the draw list for a gizmo.
///
/// `arm` is a unit cube mesh; the caller already has one and uploading a
/// second is waste.
pub fn draws(
    arm: MeshHandle,
    camera: &Camera,
    style: &GizmoStyle,
    origin: Vec3,
    active: Option<Handle>,
) -> Vec<Draw> {
    let length = style.arm_length(camera, origin);
    let thick = length * style.thickness;
    Handle::ALL
        .iter()
        .map(|handle| {
            let axis = handle.axis();
            // A box from the origin outward along the axis, so the arm grows
            // from the object rather than being centred on it.
            let scale = axis * length + (Vec3::ONE - axis) * thick;
            let transform =
                Mat4::from_translation(origin + axis * length * 0.5) * Mat4::from_scale(scale);
            let mut material = handle.color();
            if active == Some(*handle) {
                // The held one goes white, because a colour that merely
                // brightens is hard to tell apart from the light changing.
                material.base_color = [1.0, 1.0, 1.0];
            }
            Draw {
                mesh: arm,
                transform,
                texture: TextureHandle::WHITE,
                material,
                pose: None,
            }
        })
        .collect()
}

/// Which handle a ray passes close enough to, nearest first.
pub fn hit(
    camera: &Camera,
    style: &GizmoStyle,
    origin: Vec3,
    ray_origin: Vec3,
    ray_direction: Vec3,
) -> Option<Handle> {
    let length = style.arm_length(camera, origin);
    let slack = length * style.grab_slack;

    let mut best: Option<(f32, f32, Handle)> = None;
    for handle in Handle::ALL {
        let axis = handle.axis();
        let Some((along, distance)) = closest_points(origin, axis, ray_origin, ray_direction)
        else {
            continue;
        };
        // Only the arm itself, not the infinite line it lies on.
        if !(0.0..=length).contains(&along) || distance > slack {
            continue;
        }
        let depth = (origin + axis * along - ray_origin).length();
        if best.is_none_or(|(closest, _, _)| depth < closest) {
            best = Some((depth, distance, handle));
        }
    }
    best.map(|(_, _, handle)| handle)
}

/// Start a drag on a handle.
pub fn begin(origin: Vec3, handle: Handle, ray_origin: Vec3, ray_direction: Vec3) -> Drag {
    let grab_offset = closest_points(origin, handle.axis(), ray_origin, ray_direction)
        .map(|(along, _)| along)
        // A ray exactly along the axis has no unique closest point. Treating
        // the grab as being at the origin is wrong by less than the width of
        // the handle, and the alternative is refusing to start a drag that
        // the user clearly asked for.
        .unwrap_or(0.0);
    Drag {
        handle,
        origin,
        grab_offset,
    }
}

/// Where the gizmo should be now, given where the cursor's ray is.
///
/// Returns the new position rather than a delta: a caller that accumulates
/// deltas accumulates their rounding too, and a long drag drifts.
pub fn update(drag: &Drag, ray_origin: Vec3, ray_direction: Vec3) -> Vec3 {
    let axis = drag.handle.axis();
    let Some((along, _)) = closest_points(drag.origin, axis, ray_origin, ray_direction) else {
        // Edge-on: the axis and the ray are parallel and there is no answer.
        // Staying put is the only sane one; moving by a guess is how a
        // gizmo flings an object off screen.
        return drag.origin;
    };
    drag.origin + axis * (along - drag.grab_offset)
}

/// Where a line and a ray come closest: how far along the line, and how far
/// apart they are there.
///
/// `None` when they are parallel, which has no answer rather than a large
/// one.
fn closest_points(
    line_origin: Vec3,
    line_direction: Vec3,
    ray_origin: Vec3,
    ray_direction: Vec3,
) -> Option<(f32, f32)> {
    let (u, v) = (line_direction, ray_direction.normalize_or_zero());
    let w = line_origin - ray_origin;
    let b = u.dot(v);
    let denominator = 1.0 - b * b;
    if denominator.abs() < 1e-5 {
        return None;
    }
    let (d, e) = (u.dot(w), v.dot(w));
    let along = (b * e - d) / denominator;
    let ray_at = (e - b * d) / denominator;
    let point_on_line = line_origin + u * along;
    let point_on_ray = ray_origin + v * ray_at;
    Some((along, (point_on_line - point_on_ray).length()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        Camera {
            position: Vec3::new(0.0, 0.0, 10.0),
            target: Vec3::ZERO,
            ..Camera::default()
        }
    }

    #[test]
    fn a_gizmo_keeps_its_size_on_screen_as_it_recedes() {
        // Fixed world size would make a gizmo on a distant object a dot and
        // one on a near object fill the view.
        let style = GizmoStyle::default();
        let near = style.arm_length(&camera(), Vec3::new(0.0, 0.0, 9.0));
        let far = style.arm_length(&camera(), Vec3::new(0.0, 0.0, -90.0));
        assert!(far > near * 10.0, "near {near}, far {far}");
    }

    #[test]
    fn a_ray_down_an_arm_picks_that_arm() {
        let style = GizmoStyle::default();
        let origin = Vec3::ZERO;
        let length = style.arm_length(&camera(), origin);
        // Aimed at the middle of the X arm, from in front.
        let from = Vec3::new(length * 0.5, 0.0, 5.0);
        assert_eq!(
            hit(&camera(), &style, origin, from, Vec3::NEG_Z),
            Some(Handle::X)
        );
    }

    #[test]
    fn a_ray_past_the_end_of_an_arm_picks_nothing() {
        // Only the arm, not the infinite line it lies on — otherwise the
        // whole screen is a handle.
        let style = GizmoStyle::default();
        let length = style.arm_length(&camera(), Vec3::ZERO);
        let beyond = Vec3::new(length * 3.0, 0.0, 5.0);
        assert_eq!(
            hit(&camera(), &style, Vec3::ZERO, beyond, Vec3::NEG_Z),
            None
        );
    }

    #[test]
    fn a_drag_does_not_jump_the_object_to_the_cursor() {
        // Grabbing the far end of an arm and not moving must leave the
        // object exactly where it was. Following the cursor directly
        // teleports it by the length of the arm on the first frame.
        let style = GizmoStyle::default();
        let origin = Vec3::new(1.0, 2.0, 3.0);
        let length = style.arm_length(&camera(), origin);
        let grab_at = origin + Vec3::X * length * 0.9;
        let from = grab_at + Vec3::Z * 5.0;

        let drag = begin(origin, Handle::X, from, Vec3::NEG_Z);
        let moved = update(&drag, from, Vec3::NEG_Z);
        assert!(
            (moved - origin).length() < 1e-4,
            "expected no movement, got {moved}"
        );
    }

    #[test]
    fn a_drag_moves_along_its_own_axis_and_no_other() {
        let style = GizmoStyle::default();
        let origin = Vec3::ZERO;
        let length = style.arm_length(&camera(), origin);
        let from = Vec3::new(length * 0.5, 0.0, 5.0);
        let drag = begin(origin, Handle::X, from, Vec3::NEG_Z);

        // Cursor moves two metres right and one metre up; only the right
        // part may count.
        let moved_ray = from + Vec3::new(2.0, 1.0, 0.0);
        let moved = update(&drag, moved_ray, Vec3::NEG_Z);
        assert!((moved.x - 2.0).abs() < 1e-3, "got {moved}");
        assert!(moved.y.abs() < 1e-4, "y must not move: {moved}");
        assert!(moved.z.abs() < 1e-4, "z must not move: {moved}");
    }

    #[test]
    fn an_edge_on_axis_stays_put_rather_than_flying_off() {
        // Looking straight down the X axis, a drag on X has no answer. A
        // guess here is how a gizmo throws an object out of the scene.
        let drag = Drag {
            handle: Handle::X,
            origin: Vec3::ZERO,
            grab_offset: 0.0,
        };
        let moved = update(&drag, Vec3::new(-10.0, 0.0, 0.0), Vec3::X);
        assert_eq!(moved, Vec3::ZERO);
    }

    #[test]
    fn the_held_handle_is_told_apart_from_the_others() {
        let style = GizmoStyle::default();
        let list = draws(
            MeshHandle::TEST,
            &camera(),
            &style,
            Vec3::ZERO,
            Some(Handle::Y),
        );
        assert_eq!(list.len(), 3);
        assert_eq!(list[1].material.base_color, [1.0, 1.0, 1.0]);
        assert_ne!(list[0].material.base_color, [1.0, 1.0, 1.0]);
        // And none of them is lit, so a handle never sinks into a shadow.
        assert!(list.iter().all(|d| d.material.shading == Shading::Unlit));
    }
}
