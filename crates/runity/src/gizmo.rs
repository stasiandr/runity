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

use glam::{Mat4, Quat, Vec3};

use crate::material::{Material, Shading};
use crate::render::{Camera, Draw, MeshHandle, TextureHandle};

/// What the gizmo does with a drag.
///
/// Three tools rather than three gizmos, because everything but the solver
/// is the same: the same three axes, the same colours, the same rule about
/// staying the same size on screen.
///
/// All three solve in **world** axes. Handles along an entity's own axes
/// are the same solvers in a turned frame — [`ray_into`], [`draws_turned`],
/// [`motion_out_of`] — rather than a second set of cases, so what is drawn
/// and what a drag does cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// Arms along the axes; a drag slides along one.
    #[default]
    Move,
    /// Rings around the axes; a drag turns about one.
    Rotate,
    /// Arms with a box at the end; a drag stretches along one.
    Scale,
}

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
        let distance = camera.apparent_distance(origin).max(0.01);
        distance * self.screen_size
    }
}

/// A grab in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drag {
    pub tool: Tool,
    pub handle: Handle,
    /// Where the gizmo was when the grab started.
    pub origin: Vec3,
    /// Where on the handle the grab landed, so nothing jumps on the first
    /// frame: how far along the axis for move and scale, the angle around
    /// the ring for rotate.
    pub grab_offset: f32,
}

/// What a drag asks the caller to do.
///
/// Relative to the transform the drag *started* from, never to the last
/// frame. A caller that accumulated per-frame deltas would accumulate their
/// rounding too, and a long drag would drift — the same reason the move
/// solver returns a position rather than a step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Motion {
    /// Where the gizmo should be now.
    Position(Vec3),
    /// Turn the starting rotation by this, in world axes.
    Rotation(Quat),
    /// Multiply the starting scale by this, per axis.
    Scale(Vec3),
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
    draws_for(Tool::Move, arm, camera, style, origin, active)
}

/// Build the draw list for one of the three tools.
///
/// Every one is made of the same unit cube: rings are a ring of short boxes.
/// A circle mesh would be prettier and would be a second thing to upload,
/// keep and hand around for handles that are a few dozen boxes on screen.
pub fn draws_for(
    tool: Tool,
    arm: MeshHandle,
    camera: &Camera,
    style: &GizmoStyle,
    origin: Vec3,
    active: Option<Handle>,
) -> Vec<Draw> {
    match tool {
        Tool::Move => arms(arm, camera, style, origin, active, false),
        Tool::Scale => arms(arm, camera, style, origin, active, true),
        Tool::Rotate => rings(arm, camera, style, origin, active),
    }
}

/// The colour a handle is drawn in, white while it is the one being used.
fn handle_material(handle: Handle, active: Option<Handle>) -> Material {
    let mut material = handle.color();
    if active == Some(handle) {
        // The held one goes white, because a colour that merely brightens is
        // hard to tell apart from the light changing.
        material.base_color = [1.0, 1.0, 1.0];
    }
    material
}

/// Three arms along the axes, with a box on the end for the scale tool.
fn arms(
    arm: MeshHandle,
    camera: &Camera,
    style: &GizmoStyle,
    origin: Vec3,
    active: Option<Handle>,
    knobs: bool,
) -> Vec<Draw> {
    let length = style.arm_length(camera, origin);
    let thick = length * style.thickness;
    let mut out = Vec::with_capacity(if knobs { 6 } else { 3 });
    for handle in Handle::ALL {
        let axis = handle.axis();
        // A box from the origin outward along the axis, so the arm grows
        // from the object rather than being centred on it.
        let scale = axis * length + (Vec3::ONE - axis) * thick;
        let transform =
            Mat4::from_translation(origin + axis * length * 0.5) * Mat4::from_scale(scale);
        let material = handle_material(handle, active);
        out.push(Draw {
            mesh: arm,
            transform,
            texture: TextureHandle::WHITE,
            material,
            pose: None,
        });
        if knobs {
            // A cube on the end, which is what tells scale apart from move
            // at a glance — the arms alone are identical.
            let knob = thick * 3.0;
            out.push(Draw {
                mesh: arm,
                transform: Mat4::from_translation(origin + axis * length)
                    * Mat4::from_scale(Vec3::splat(knob)),
                texture: TextureHandle::WHITE,
                material,
                pose: None,
            });
        }
    }
    out
}

/// How many boxes a ring is made of. Twenty-four reads as a circle at the
/// size a gizmo is drawn and costs nothing worth counting.
const RING_SEGMENTS: usize = 24;

/// Three rings, one around each axis.
fn rings(
    arm: MeshHandle,
    camera: &Camera,
    style: &GizmoStyle,
    origin: Vec3,
    active: Option<Handle>,
) -> Vec<Draw> {
    let radius = style.arm_length(camera, origin);
    let thick = radius * style.thickness;
    let mut out = Vec::with_capacity(Handle::ALL.len() * RING_SEGMENTS);
    for handle in Handle::ALL {
        let (u, v) = ring_plane(handle);
        let material = handle_material(handle, active);
        for segment in 0..RING_SEGMENTS {
            let step = std::f32::consts::TAU / RING_SEGMENTS as f32;
            let (a, b) = (segment as f32 * step, (segment + 1) as f32 * step);
            let from = origin + (u * a.cos() + v * a.sin()) * radius;
            let to = origin + (u * b.cos() + v * b.sin()) * radius;
            let along = to - from;
            let rotation = Quat::from_rotation_arc(Vec3::X, along.normalize_or_zero());
            out.push(Draw {
                mesh: arm,
                transform: Mat4::from_scale_rotation_translation(
                    Vec3::new(along.length(), thick, thick),
                    rotation,
                    (from + to) * 0.5,
                ),
                texture: TextureHandle::WHITE,
                material,
                pose: None,
            });
        }
    }
    out
}

/// The two axes a handle's ring lies in.
///
/// The ring for X is the one you turn *about* X, so it lies in the YZ plane.
/// Getting this backwards draws three rings that look right and turn the
/// wrong way, which is why the pair is named once and used everywhere.
fn ring_plane(handle: Handle) -> (Vec3, Vec3) {
    match handle {
        Handle::X => (Vec3::Y, Vec3::Z),
        Handle::Y => (Vec3::Z, Vec3::X),
        Handle::Z => (Vec3::X, Vec3::Y),
    }
}

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
    };
    Material::new(r, g, b).unlit()
}

/// What is selected is outlined in this: Unity's orange.
pub fn selection_color() -> Material {
    Material::new(1.0, 0.42, 0.0).unlit()
}

/// A model's box as lines — `min` and `max` in its own space, `placed` its
/// world matrix, scale and all: the outline around a selected thing.
pub fn bounds_draws(
    arm: MeshHandle,
    min: Vec3,
    max: Vec3,
    placed: Mat4,
    thickness: f32,
    material: Material,
) -> Vec<Draw> {
    // Unscaled: the placing matrix's scale is applied as for a collider.
    let half = ((max - min) * 0.5).max(Vec3::splat(1e-4));
    let centre = placed * Mat4::from_translation((min + max) * 0.5);
    collider_draws(
        arm,
        crate::scene::Collider::Box { half },
        centre,
        thickness,
        material,
    )
}

/// A collider as lines: the shape physics sees, drawn over what the eye
/// sees, so a crate whose box is half a metre off is visible as that and
/// not discovered by walking into air. The same sizes the physics world
/// builds — sphere radius by the largest scale, capsule and cylinder by
/// the larger of x and z — so what is drawn is what collides. A `Model`
/// collider draws nothing here; it is the model.
///
/// `arm` is a unit cube, as for the handles; `thickness` is a line's width
/// in metres.
pub fn collider_draws(
    arm: MeshHandle,
    shape: crate::scene::Collider,
    placed: Mat4,
    thickness: f32,
    material: Material,
) -> Vec<Draw> {
    use crate::scene::Collider;
    let (scale, rotation, translation) = placed.to_scale_rotation_translation();
    // Placed without scale: sizes below are already scaled, as physics
    // scales them.
    let frame = Mat4::from_rotation_translation(rotation, translation);
    let mut segments: Vec<(Vec3, Vec3)> = Vec::new();
    fn circle(out: &mut Vec<(Vec3, Vec3)>, centre: Vec3, u: Vec3, v: Vec3, radius: f32) {
        let step = std::f32::consts::TAU / RING_SEGMENTS as f32;
        for i in 0..RING_SEGMENTS {
            let (a, b) = (i as f32 * step, (i + 1) as f32 * step);
            out.push((
                centre + (u * a.cos() + v * a.sin()) * radius,
                centre + (u * b.cos() + v * b.sin()) * radius,
            ));
        }
    }
    let box_edges = |h: Vec3| -> Vec<(Vec3, Vec3)> {
        let corner = |x: f32, y: f32, z: f32| Vec3::new(x * h.x, y * h.y, z * h.z);
        let mut edges = Vec::new();
        for (a, b) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
            edges.push((corner(-1.0, a, b), corner(1.0, a, b)));
            edges.push((corner(a, -1.0, b), corner(a, 1.0, b)));
            edges.push((corner(a, b, -1.0), corner(a, b, 1.0)));
        }
        edges
    };
    // A ramp: high at the back (−z), down to nothing at the front.
    let ramp_edges = |h: Vec3| -> Vec<(Vec3, Vec3)> {
        let c = |x: f32, y: f32, z: f32| Vec3::new(x * h.x, y * h.y, z * h.z);
        vec![
            (c(-1.0, -1.0, -1.0), c(1.0, -1.0, -1.0)),
            (c(1.0, -1.0, -1.0), c(1.0, -1.0, 1.0)),
            (c(1.0, -1.0, 1.0), c(-1.0, -1.0, 1.0)),
            (c(-1.0, -1.0, 1.0), c(-1.0, -1.0, -1.0)),
            (c(-1.0, -1.0, -1.0), c(-1.0, 1.0, -1.0)),
            (c(1.0, -1.0, -1.0), c(1.0, 1.0, -1.0)),
            (c(-1.0, 1.0, -1.0), c(1.0, 1.0, -1.0)),
            (c(-1.0, 1.0, -1.0), c(-1.0, -1.0, 1.0)),
            (c(1.0, 1.0, -1.0), c(1.0, -1.0, 1.0)),
        ]
    };
    match shape {
        Collider::None | Collider::Model => return Vec::new(),
        Collider::Box { half } | Collider::Stairs { half, .. } => {
            segments.extend(box_edges(half * scale));
        }
        Collider::Ramp { half } => segments.extend(ramp_edges(half * scale)),
        Collider::Sphere { radius } => {
            let r = radius * scale.max_element();
            circle(&mut segments, Vec3::ZERO, Vec3::X, Vec3::Y, r);
            circle(&mut segments, Vec3::ZERO, Vec3::Y, Vec3::Z, r);
            circle(&mut segments, Vec3::ZERO, Vec3::Z, Vec3::X, r);
        }
        Collider::Capsule {
            half_height,
            radius,
        }
        | Collider::Cylinder {
            half_height,
            radius,
        } => {
            let (h, r) = (half_height * scale.y, radius * scale.x.max(scale.z));
            circle(&mut segments, Vec3::Y * h, Vec3::X, Vec3::Z, r);
            circle(&mut segments, -Vec3::Y * h, Vec3::X, Vec3::Z, r);
            for side in [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z] {
                segments.push((side * r - Vec3::Y * h, side * r + Vec3::Y * h));
            }
            if matches!(shape, Collider::Capsule { .. }) {
                // The caps, as half-rings over the top and under the bottom.
                circle(&mut segments, Vec3::Y * h, Vec3::X, Vec3::Y, r);
                circle(&mut segments, -Vec3::Y * h, Vec3::Z, Vec3::Y, r);
            }
        }
    }
    segments
        .into_iter()
        .filter_map(|(a, b)| {
            let (a, b) = (frame.transform_point3(a), frame.transform_point3(b));
            let along = b - a;
            let length = along.length();
            (length > 1e-5).then(|| Draw {
                mesh: arm,
                transform: Mat4::from_scale_rotation_translation(
                    Vec3::new(length + thickness, thickness, thickness),
                    Quat::from_rotation_arc(Vec3::X, along / length),
                    (a + b) * 0.5,
                ),
                texture: TextureHandle::WHITE,
                material,
                pose: None,
            })
        })
        .collect()
}

/// Handles along an entity's own axes rather than the world's: Unity's
/// Local. Rather than teach every solver a second set of cases, the ray is
/// turned into the handles' frame, the world-axis solver runs there, and
/// what it answers is turned back — one rule, so what is drawn and what a
/// drag does cannot disagree.
///
/// A ray into a frame turned by `orientation` about `origin`.
pub fn ray_into(
    origin: Vec3,
    orientation: Quat,
    ray_origin: Vec3,
    ray_direction: Vec3,
) -> (Vec3, Vec3) {
    let back = orientation.inverse();
    (origin + back * (ray_origin - origin), back * ray_direction)
}

/// Draws made in world axes, turned into the frame.
pub fn draws_turned(draws: Vec<Draw>, origin: Vec3, orientation: Quat) -> Vec<Draw> {
    let turn = Mat4::from_translation(origin)
        * Mat4::from_quat(orientation)
        * Mat4::from_translation(-origin);
    draws
        .into_iter()
        .map(|d| Draw {
            transform: turn * d.transform,
            ..d
        })
        .collect()
}

/// A solver's answer in the frame, back in the world. A scale is per the
/// frame's axes either way, which are the entity's own when the frame is
/// its rotation.
pub fn motion_out_of(motion: Motion, origin: Vec3, orientation: Quat) -> Motion {
    match motion {
        Motion::Position(p) => Motion::Position(origin + orientation * (p - origin)),
        Motion::Rotation(q) => Motion::Rotation(orientation * q * orientation.inverse()),
        Motion::Scale(s) => Motion::Scale(s),
    }
}

/// Which handle a ray passes close enough to, nearest first.
pub fn hit(
    camera: &Camera,
    style: &GizmoStyle,
    origin: Vec3,
    ray_origin: Vec3,
    ray_direction: Vec3,
) -> Option<Handle> {
    hit_for(Tool::Move, camera, style, origin, ray_origin, ray_direction)
}

/// Which handle of a given tool a ray hits.
pub fn hit_for(
    tool: Tool,
    camera: &Camera,
    style: &GizmoStyle,
    origin: Vec3,
    ray_origin: Vec3,
    ray_direction: Vec3,
) -> Option<Handle> {
    match tool {
        Tool::Move | Tool::Scale => hit_arm(camera, style, origin, ray_origin, ray_direction),
        Tool::Rotate => hit_ring(camera, style, origin, ray_origin, ray_direction),
    }
}

/// Which ring a ray crosses: the one whose circle it passes nearest to.
fn hit_ring(
    camera: &Camera,
    style: &GizmoStyle,
    origin: Vec3,
    ray_origin: Vec3,
    ray_direction: Vec3,
) -> Option<Handle> {
    let radius = style.arm_length(camera, origin);
    let slack = radius * style.grab_slack;
    let direction = ray_direction.normalize_or_zero();

    let mut best: Option<(f32, Handle)> = None;
    for handle in Handle::ALL {
        let axis = handle.axis();
        let facing = axis.dot(direction);
        if facing.abs() < 1e-4 {
            // Edge-on: the ray runs inside the ring's plane and never
            // crosses it. A ring seen edge-on is a line a pixel wide, and
            // guessing here means grabbing a ring nobody could see.
            continue;
        }
        let travel = axis.dot(origin - ray_origin) / facing;
        if travel < 0.0 {
            continue; // behind the camera
        }
        let point = ray_origin + direction * travel;
        let off_circle = ((point - origin).length() - radius).abs();
        if off_circle > slack {
            continue;
        }
        if best.is_none_or(|(closest, _)| travel < closest) {
            best = Some((travel, handle));
        }
    }
    best.map(|(_, handle)| handle)
}

fn hit_arm(
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

/// Start a move drag on a handle.
pub fn begin(origin: Vec3, handle: Handle, ray_origin: Vec3, ray_direction: Vec3) -> Drag {
    begin_for(Tool::Move, origin, handle, ray_origin, ray_direction)
}

/// Start a drag with a given tool.
pub fn begin_for(
    tool: Tool,
    origin: Vec3,
    handle: Handle,
    ray_origin: Vec3,
    ray_direction: Vec3,
) -> Drag {
    let grab_offset = match tool {
        Tool::Rotate => angle_at(origin, handle, ray_origin, ray_direction).unwrap_or(0.0),
        Tool::Move | Tool::Scale => {
            closest_points(origin, handle.axis(), ray_origin, ray_direction)
                .map(|(along, _)| along)
                // A ray exactly along the axis has no unique closest point.
                // Treating the grab as being at the origin is wrong by less than
                // the width of the handle, and the alternative is refusing to
                // start a drag the user clearly asked for.
                .unwrap_or(0.0)
        }
    };
    Drag {
        tool,
        handle,
        origin,
        grab_offset,
    }
}

/// Where the cursor's ray crosses a ring's plane, as an angle around it.
///
/// `None` when the ray runs inside the plane and never crosses it.
fn angle_at(origin: Vec3, handle: Handle, ray_origin: Vec3, ray_direction: Vec3) -> Option<f32> {
    let axis = handle.axis();
    let direction = ray_direction.normalize_or_zero();
    let facing = axis.dot(direction);
    if facing.abs() < 1e-4 {
        return None;
    }
    let travel = axis.dot(origin - ray_origin) / facing;
    let point = ray_origin + direction * travel - origin;
    let (u, v) = ring_plane(handle);
    let (x, y) = (point.dot(u), point.dot(v));
    if x.abs() < 1e-6 && y.abs() < 1e-6 {
        // Dead centre: no angle. Refusing beats spinning on a rounding
        // error at the pivot.
        return None;
    }
    Some(y.atan2(x))
}

/// What a drag asks for now, given where the cursor's ray is.
pub fn update_for(drag: &Drag, ray_origin: Vec3, ray_direction: Vec3) -> Motion {
    match drag.tool {
        Tool::Move => Motion::Position(update(drag, ray_origin, ray_direction)),
        Tool::Rotate => {
            let Some(angle) = angle_at(drag.origin, drag.handle, ray_origin, ray_direction) else {
                // Edge-on, or exactly at the pivot: no answer. Staying put
                // is the only sane one.
                return Motion::Rotation(Quat::IDENTITY);
            };
            // Wrapped into a half turn either way, so that dragging past the
            // far side of the ring keeps turning the same direction instead
            // of snapping most of the way round.
            let mut delta = angle - drag.grab_offset;
            while delta > std::f32::consts::PI {
                delta -= std::f32::consts::TAU;
            }
            while delta < -std::f32::consts::PI {
                delta += std::f32::consts::TAU;
            }
            Motion::Rotation(Quat::from_axis_angle(drag.handle.axis(), delta))
        }
        Tool::Scale => {
            let axis = drag.handle.axis();
            let Some((along, _)) = closest_points(drag.origin, axis, ray_origin, ray_direction)
            else {
                return Motion::Scale(Vec3::ONE);
            };
            // Grabbing at the origin gives nothing to divide by; so does
            // dragging through it. Both are clamped rather than refused, so
            // a drag that passes the pivot shrinks toward nothing instead of
            // turning the object inside out.
            let grabbed = drag.grab_offset.abs().max(1e-3);
            let factor = (along / grabbed).max(0.01);
            Motion::Scale(Vec3::ONE + axis * (factor - 1.0))
        }
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

/// Round a value to the nearest multiple of `step`, or leave it alone when
/// `step` is zero or less.
///
/// The thing snapping is for: a wall laid out by eye is a wall with a
/// half-centimetre gap in it, and the gap is invisible until something walks
/// through it. Applied to the *result* rather than to the movement, so a
/// drag lands on the grid rather than on wherever it started plus a whole
/// number of steps.
pub fn snap(value: f32, step: f32) -> f32 {
    if step <= 0.0 || !step.is_finite() {
        return value;
    }
    (value / step).round() * step
}

/// Snap each component of a vector.
pub fn snap_all(value: Vec3, step: f32) -> Vec3 {
    Vec3::new(
        snap(value.x, step),
        snap(value.y, step),
        snap(value.z, step),
    )
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
    fn turned_handles_slide_along_the_turned_axis() {
        // From the side: the turned x runs across the view, not into it.
        let camera = Camera {
            position: Vec3::new(10.0, 3.0, 0.0),
            target: Vec3::ZERO,
            ..Camera::default()
        };
        let style = GizmoStyle::default();
        let origin = Vec3::ZERO;
        // A quarter turn about y: the entity's x is the world's −z.
        let turn = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let length = style.arm_length(&camera, origin);
        let aim = |p: Vec3| {
            let direction = (p - camera.position).normalize();
            ray_into(origin, turn, camera.position, direction)
        };
        let (from, direction) = aim(turn * Vec3::X * length * 0.7);
        let handle = hit_for(Tool::Move, &camera, &style, origin, from, direction);
        assert_eq!(handle, Some(Handle::X));
        let drag = begin_for(Tool::Move, origin, Handle::X, from, direction);
        let (from, direction) = aim(turn * Vec3::X * length * 1.7);
        let Motion::Position(p) = motion_out_of(update_for(&drag, from, direction), origin, turn)
        else {
            panic!("a move");
        };
        assert!(p.z < -0.1 && p.x.abs() < 1e-3 && p.y.abs() < 1e-3, "{p}");
        let drawn = draws_turned(
            draws(MeshHandle::TEST, &camera, &style, origin, None),
            origin,
            turn,
        );
        let x_arm = drawn[0].transform.w_axis.truncate();
        assert!(
            x_arm.z < 0.0 && x_arm.x.abs() < 1e-3,
            "drawn where it is grabbed: {x_arm}"
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
            tool: Tool::Move,
            handle: Handle::X,
            origin: Vec3::ZERO,
            grab_offset: 0.0,
        };
        let moved = update(&drag, Vec3::new(-10.0, 0.0, 0.0), Vec3::X);
        assert_eq!(moved, Vec3::ZERO);
    }

    /// Degrees of turn a rotate drag asks for, for readability below.
    fn turned(motion: Motion) -> f32 {
        match motion {
            Motion::Rotation(q) => {
                let (axis, angle) = q.to_axis_angle();
                // `to_axis_angle` may hand back the opposite axis with the
                // opposite angle; measured against Y so the sign means the
                // same thing in every case here.
                angle.to_degrees() * axis.dot(Vec3::Y).signum()
            }
            other => panic!("expected a rotation, got {other:?}"),
        }
    }

    #[test]
    fn a_ring_is_grabbed_where_the_ray_crosses_its_plane() {
        // The Y ring lies flat, so a ray coming down from above crosses it.
        // Aimed at the rim, not at the middle: the middle of a ring is the
        // object, and grabbing it there would make the whole gizmo a handle.
        let style = GizmoStyle::default();
        let origin = Vec3::ZERO;
        let radius = style.arm_length(&camera(), origin);
        let above = Vec3::new(radius, 4.0, 0.0);
        assert_eq!(
            hit_for(Tool::Rotate, &camera(), &style, origin, above, Vec3::NEG_Y),
            Some(Handle::Y)
        );

        // Through the middle, well inside the rim: nothing.
        assert_eq!(
            hit_for(
                Tool::Rotate,
                &camera(),
                &style,
                origin,
                Vec3::new(0.0, 4.0, 0.0),
                Vec3::NEG_Y
            ),
            None
        );
    }

    #[test]
    fn a_turn_is_measured_from_where_the_ring_was_grabbed() {
        // Grab the Y ring at its +X point and drag to its +Z point: a
        // quarter turn, and not a jump on the first frame.
        let style = GizmoStyle::default();
        let origin = Vec3::ZERO;
        let radius = style.arm_length(&camera(), origin);
        let grab = Vec3::new(radius, 4.0, 0.0);
        let drag = begin_for(Tool::Rotate, origin, Handle::Y, grab, Vec3::NEG_Y);

        assert!(
            turned(update_for(&drag, grab, Vec3::NEG_Y)).abs() < 1e-3,
            "a grab with no movement must not turn anything"
        );

        let quarter = Vec3::new(0.0, 4.0, radius);
        let angle = turned(update_for(&drag, quarter, Vec3::NEG_Y));
        assert!(
            (angle.abs() - 90.0).abs() < 1.0,
            "expected a quarter turn, got {angle}"
        );
    }

    #[test]
    fn dragging_past_the_far_side_keeps_turning_the_same_way() {
        // The angle wraps at pi. Without unwrapping, a drag a little past
        // half a turn snaps back the long way round — which reads as the
        // object flipping.
        let style = GizmoStyle::default();
        let radius = style.arm_length(&camera(), Vec3::ZERO);
        let grab = Vec3::new(radius, 4.0, 0.0);
        let drag = begin_for(Tool::Rotate, Vec3::ZERO, Handle::Y, grab, Vec3::NEG_Y);

        let mut previous: f32 = 0.0;
        for step in 1..=8 {
            let angle = step as f32 * std::f32::consts::TAU / 16.0;
            let point = Vec3::new(radius * angle.cos(), 4.0, -radius * angle.sin());
            let turn = turned(update_for(&drag, point, Vec3::NEG_Y));
            assert!(
                turn.abs() >= previous.abs() - 1.0,
                "the turn went backwards at step {step}: {previous} then {turn}"
            );
            previous = turn;
        }
    }

    #[test]
    fn a_scale_drag_stretches_by_the_ratio_it_was_dragged() {
        let style = GizmoStyle::default();
        let length = style.arm_length(&camera(), Vec3::ZERO);
        let from = Vec3::new(length, 0.0, 5.0);
        let drag = begin_for(Tool::Scale, Vec3::ZERO, Handle::X, from, Vec3::NEG_Z);

        assert_eq!(
            update_for(&drag, from, Vec3::NEG_Z),
            Motion::Scale(Vec3::ONE),
            "no movement, no stretch"
        );

        let doubled = from + Vec3::X * length;
        match update_for(&drag, doubled, Vec3::NEG_Z) {
            Motion::Scale(factor) => {
                assert!((factor.x - 2.0).abs() < 1e-3, "got {factor}");
                assert_eq!((factor.y, factor.z), (1.0, 1.0), "one axis only");
            }
            other => panic!("expected a scale, got {other:?}"),
        }
    }

    #[test]
    fn dragging_a_scale_handle_through_the_pivot_shrinks_rather_than_inverts() {
        // A negative factor turns an object inside out: back faces forward,
        // lighting wrong, and no obvious way back. Clamped to very small
        // instead.
        let style = GizmoStyle::default();
        let length = style.arm_length(&camera(), Vec3::ZERO);
        let from = Vec3::new(length, 0.0, 5.0);
        let drag = begin_for(Tool::Scale, Vec3::ZERO, Handle::X, from, Vec3::NEG_Z);
        let past = from - Vec3::X * length * 3.0;
        match update_for(&drag, past, Vec3::NEG_Z) {
            Motion::Scale(factor) => assert!(factor.x > 0.0 && factor.x < 0.1, "got {factor}"),
            other => panic!("expected a scale, got {other:?}"),
        }
    }

    #[test]
    fn each_tool_draws_its_own_handles() {
        let style = GizmoStyle::default();
        let at = |tool| draws_for(tool, MeshHandle::TEST, &camera(), &style, Vec3::ZERO, None);
        assert_eq!(at(Tool::Move).len(), 3, "three arms");
        assert_eq!(at(Tool::Scale).len(), 6, "three arms and three knobs");
        assert_eq!(at(Tool::Rotate).len(), 3 * RING_SEGMENTS, "three rings");
        for tool in [Tool::Move, Tool::Rotate, Tool::Scale] {
            assert!(
                at(tool)
                    .iter()
                    .all(|d| d.material.shading == Shading::Unlit),
                "{tool:?} handles must not sink into a shadow"
            );
        }
    }

    #[test]
    fn snapping_lands_on_the_grid_and_not_on_a_multiple_of_where_it_started() {
        // Snapping the movement rather than the result is the version that
        // feels right for one drag and leaves everything off-grid by the
        // same stubborn fraction forever.
        assert_eq!(snap(1.13, 0.25), 1.25);
        assert_eq!(snap(-1.13, 0.25), -1.25);
        assert_eq!(snap(0.1, 0.25), 0.0);
        assert_eq!(
            snap_all(Vec3::new(0.3, 2.61, -0.9), 0.5),
            Vec3::new(0.5, 2.5, -1.0)
        );
        // Off, and nonsense, leave the value alone rather than making it
        // NaN — a snap of zero is what "no snapping" is spelled as.
        assert_eq!(snap(1.37, 0.0), 1.37);
        assert_eq!(snap(1.37, -1.0), 1.37);
        assert_eq!(snap(1.37, f32::NAN), 1.37);
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
