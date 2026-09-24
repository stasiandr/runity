//! The handles you drag to move, turn, stretch and resize something:
//! Unity's, piece for piece.
//!
//! Three parts, and the third is the one that is easy to get wrong:
//!
//! * a draw list, so the handles appear;
//! * a hit test, so a click picks one;
//! * a drag solver, which turns mouse movement into motion.
//!
//! The first two come from one list of parts — each handle's pieces to
//! draw and the shapes it is grabbed by, made together from the same
//! numbers — so a handle is grabbable exactly where it is drawn, and a new
//! one cannot be drawn in one place and grabbed in another.
//!
//! The draws belong in [`crate::render::Frame::overlay_draws`], which is
//! drawn with the depth test off. A gizmo half inside the object it sits on
//! cannot be grabbed by anyone who cannot see it, and burying it is exactly
//! what depth testing does. With no depth, what is drawn later is on top:
//! the parts are drawn far to near, fills under lines, the middle last.
//!
//! The solver does not follow the cursor. It finds the point on the axis
//! (the plane, the ring, the ball) closest to the ray under the cursor, and
//! moves the object so that the point first grabbed stays under the cursor.
//! Following the cursor directly makes the object jump to the pointer the
//! instant it is grabbed, and makes a near-edge-on axis fly off — which is
//! the whole reason this is written down rather than done by feel.
//!
//! Sizes are fractions of the arm's length ([`GizmoStyle::arm_length`]),
//! which is a fraction of the distance: the gizmo is the same size on
//! screen wherever it is, as Unity's `HandleUtility.GetHandleSize` keeps it.

use glam::{Mat4, Quat, Vec3};

use crate::material::{Material, Shading, SurfaceType};
use crate::render::{Camera, Draw, MeshHandle, TextureHandle};

/// What the gizmo does with a drag.
///
/// Tools rather than gizmos, because most of it is shared: the same three
/// axes, the same colours, the same rule about staying the same size on
/// screen.
///
/// All of them solve in **world** axes. Handles along an entity's own axes
/// are the same solvers in a turned frame — [`ray_into`], [`draws_turned`],
/// [`motion_out_of`], or a [`Gizmo`] with an orientation, which does the
/// turning itself — rather than a second set of cases, so what is drawn
/// and what a drag does cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tool {
    /// Arrows along the axes, squares between them, a square in the
    /// middle: a drag slides along one axis, two, or the view's plane.
    #[default]
    Move,
    /// A ring about each axis (its near half), one about the line of
    /// sight, a ball inside: a drag turns about one, or freely.
    Rotate,
    /// Arms with a cube at the end, a cube in the middle: a drag stretches
    /// one axis, or all.
    Scale,
    /// Move's arrows, Rotate's rings and Scale's cubes at once, each doing
    /// its own tool's drag: Unity's Transform tool.
    Transform,
    /// A rectangle round the selection on the plane that faces the view
    /// most: a corner or an edge resizes it with the far side staying put,
    /// the inside moves it. Unity's Rect tool.
    Rect,
}

/// Which handle is being pointed at or held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Handle {
    X,
    Y,
    Z,
    /// The square across an axis, sliding in the other two: `PlaneZ` is
    /// the blue square in the XY plane, as Unity colours it.
    PlaneX,
    PlaneY,
    PlaneZ,
    /// The middle: slides in the view's plane, or stretches every axis.
    Center,
    /// The outer ring: turns about the line of sight.
    View,
    /// Inside the rings: turns freely, a trackball.
    Free,
    /// A rect tool's handle, by its side along the rectangle's two axes,
    /// each −1, 0 or 1: a corner, an edge, or (0, 0) the inside.
    Rect(i8, i8),
}

impl Handle {
    /// The axis a handle slides along or turns about; for a plane
    /// handle, the axis across it. Zero for the rest.
    pub fn axis(self) -> Vec3 {
        match self {
            Handle::X | Handle::PlaneX => Vec3::X,
            Handle::Y | Handle::PlaneY => Vec3::Y,
            Handle::Z | Handle::PlaneZ => Vec3::Z,
            _ => Vec3::ZERO,
        }
    }

    /// Unity's handle colours, which are sRGB: red, green and blue for the
    /// axes (a plane square in its normal's), a light grey for the middle
    /// and the view's ring, the rect tool's dots blue. The convention every
    /// tool shares, and one worth not being clever about.
    pub fn color(self) -> Material {
        let (r, g, b) = match self {
            Handle::X | Handle::PlaneX => (219, 62, 29),
            Handle::Y | Handle::PlaneY => (154, 243, 72),
            Handle::Z | Handle::PlaneZ => (58, 122, 248),
            Handle::Center | Handle::View | Handle::Free => (204, 204, 204),
            Handle::Rect(0, 0) => (225, 225, 225),
            Handle::Rect(..) => (58, 122, 248),
        };
        // Unlit, so a handle is the same colour whatever the sun is
        // doing and never disappears into a shadow.
        Material::from_srgb(r, g, b).unlit()
    }

    pub const ALL: [Handle; 3] = [Handle::X, Handle::Y, Handle::Z];

    /// The axis handle for an axis index.
    fn along(index: usize) -> Handle {
        [Handle::X, Handle::Y, Handle::Z][index.min(2)]
    }

    /// The plane handle across an axis index.
    fn across(index: usize) -> Handle {
        [Handle::PlaneX, Handle::PlaneY, Handle::PlaneZ][index.min(2)]
    }
}

/// A handle and what grabbing it does. The same X is an arrow to Move, a
/// ring to Rotate and a cube to Scale, and the Transform tool shows all
/// three: `tool` is the drag it starts, never [`Tool::Transform`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Grip {
    pub tool: Tool,
    pub handle: Handle,
}

impl Grip {
    pub const fn new(tool: Tool, handle: Handle) -> Self {
        Self { tool, handle }
    }
}

/// Unity's yellow: the handle under the cursor, and the one being held.
pub fn hot_color() -> Material {
    Material::from_srgb(246, 242, 50).unlit()
}

/// How big the handles are and how forgiving they are to click.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GizmoStyle {
    /// Length of an arm, as a fraction of its distance from the camera.
    ///
    /// Scaled with distance rather than fixed in world units, so a gizmo is
    /// the same size on screen whether it is on a pebble or a mountain.
    pub screen_size: f32,
    /// Width of a line, as a fraction of an arm's length: about two
    /// pixels on a 2× screen at the default size.
    pub thickness: f32,
    /// How close a ray has to pass to a line to count as a hit, as a
    /// fraction of the arm's length. Generous, but not so generous that
    /// the arrows and squares round the middle steal each other's clicks.
    pub grab_slack: f32,
}

impl Default for GizmoStyle {
    fn default() -> Self {
        Self {
            screen_size: 0.15,
            thickness: 0.02,
            grab_slack: 0.06,
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

/// The meshes handles are made of, uploaded once by whoever draws them: a
/// unit cube (lines, cubes), a cone (arrowheads) and a triangle (squares,
/// discs, the swept angle — any triangle is the one triangle, placed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GizmoMeshes {
    pub cube: MeshHandle,
    pub cone: MeshHandle,
    pub triangle: MeshHandle,
}

/// The arrowhead's radius for its length. Unity's cones are about this
/// slender.
const CONE_ASPECT: f32 = 0.28;

impl GizmoMeshes {
    /// For tests that look at draws, not at pixels.
    pub const TEST: GizmoMeshes = GizmoMeshes {
        cube: MeshHandle::TEST,
        cone: MeshHandle::TEST,
        triangle: MeshHandle::TEST,
    };

    /// The arrowhead: base at the origin, tip at +y 1, smooth sides. Its
    /// shape is made in the mesh rather than by a stretched transform,
    /// because a stretched transform bends the normals and the shading
    /// with them.
    pub fn cone_mesh() -> crate::asset::MeshAsset {
        use crate::asset::Vertex;
        let (segments, radius) = (20u32, CONE_ASPECT);
        let tau = std::f32::consts::TAU;
        let slope = (radius * radius + 1.0).sqrt();
        let normal = |a: f32| [a.cos() / slope, radius / slope, a.sin() / slope];
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        for i in 0..segments {
            let (a0, a1) = (
                tau * i as f32 / segments as f32,
                tau * (i + 1) as f32 / segments as f32,
            );
            let base = vertices.len() as u32;
            let rim = |a: f32| [radius * a.cos(), 0.0, radius * a.sin()];
            for (position, n) in [
                (rim(a0), normal(a0)),
                (rim(a1), normal(a1)),
                ([0.0, 1.0, 0.0], normal((a0 + a1) * 0.5)),
            ] {
                vertices.push(Vertex {
                    position,
                    normal: n,
                    uv: [0.5, 0.5],
                });
            }
            indices.extend_from_slice(&[base, base + 2, base + 1]);
        }
        let centre = vertices.len() as u32;
        vertices.push(Vertex {
            position: [0.0; 3],
            normal: [0.0, -1.0, 0.0],
            uv: [0.5, 0.5],
        });
        for i in 0..segments {
            let a = tau * i as f32 / segments as f32;
            vertices.push(Vertex {
                position: [radius * a.cos(), 0.0, radius * a.sin()],
                normal: [0.0, -1.0, 0.0],
                uv: [0.5, 0.5],
            });
        }
        for i in 0..segments {
            indices.extend_from_slice(&[centre, centre + 1 + i, centre + 1 + (i + 1) % segments]);
        }
        scrap_geometry::builtin::finish("gizmo cone", vertices, indices)
    }

    /// One triangle, (0,0,0), (1,0,0), (0,1,0), wound both ways so it is
    /// seen from either side.
    pub fn triangle_mesh() -> crate::asset::MeshAsset {
        use crate::asset::Vertex;
        let vertex = |x: f32, y: f32| Vertex {
            position: [x, y, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [x, y],
        };
        scrap_geometry::builtin::finish(
            "gizmo triangle",
            vec![vertex(0.0, 0.0), vertex(1.0, 0.0), vertex(0.0, 1.0)],
            vec![0, 1, 2, 0, 2, 1],
        )
    }
}

/// What the handles show besides themselves: the one under the cursor,
/// the one held, and how far a held one has gone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shown {
    /// Under the cursor: yellow, while nothing is held.
    pub hovered: Option<Grip>,
    /// Held: yellow until let go.
    pub held: Option<Grip>,
    /// A scale drag's factor per axis, in the handles' frame: the arms
    /// stretch with it, as Unity's do.
    pub stretch: Vec3,
    /// A turn in progress: the pie it has swept.
    pub swept: Option<Swept>,
}

impl Default for Shown {
    fn default() -> Self {
        Self {
            hovered: None,
            held: None,
            stretch: Vec3::ONE,
            swept: None,
        }
    }
}

/// How far a ring has been turned, drawn as a filled pie from where it was
/// grabbed: about `axis`, from the direction `from`, by `angle` radians, at
/// the ring's `radius` — in the handles' frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Swept {
    pub handle: Handle,
    pub axis: Vec3,
    pub from: Vec3,
    pub angle: f32,
    pub radius: f32,
}

/// A grab in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drag {
    /// The drag it does: Move, Rotate, Scale or Rect.
    pub tool: Tool,
    pub handle: Handle,
    /// Where the gizmo was when the grab started.
    pub origin: Vec3,
    /// Which way the handles pointed: a drag solves in their frame.
    pub orientation: Quat,
    /// Where on the handle the grab landed, so nothing jumps on the first
    /// frame: how far along the axis for an arm, the angle round the ring
    /// for a ring.
    pub grab_offset: f32,
    /// The point grabbed, in the handles' frame: on the plane for a plane
    /// or the middle, on the ring, on the ball.
    grab_point: Vec3,
    /// Which way the view looked at the gizmo, and its right and up, in the
    /// frame, when the grab began: the middle and the view's ring solve
    /// against the view as it was, not as it is.
    toward: Vec3,
    right: Vec3,
    up: Vec3,
    /// The arm's length at the grab: the ball's radius, a stretch's unit.
    length: f32,
    /// A ring seen nearly edge-on is turned by dragging along its tangent
    /// at the grab rather than round its plane, which a ray crosses at a
    /// point that swings wildly for a pixel's move.
    tangent: Option<Vec3>,
    /// A rect handle's rectangle.
    rect: Option<RectGrab>,
}

/// What a rect handle's drag needs of the rectangle: the axis across it,
/// the two along it, a point on its plane, and the side that stays put.
#[derive(Debug, Clone, Copy, PartialEq)]
struct RectGrab {
    across: usize,
    u: usize,
    v: usize,
    plane: Vec3,
    anchor: Vec3,
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
    /// Multiply the starting scale by `scale` per axis of the handles'
    /// frame, keeping the world point `about` where it is: a rect handle,
    /// whose far side stays put.
    Resize { scale: Vec3, about: Vec3 },
}

/// A gizmo as it stands: which tool, seen from where, at what, turned
/// how. Everything the draws, the hit test and a grab need, so the three
/// are made from one description.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gizmo {
    pub tool: Tool,
    pub camera: Camera,
    pub style: GizmoStyle,
    pub origin: Vec3,
    /// The handles' frame: identity for the world's axes, an entity's
    /// rotation for its own.
    pub orientation: Quat,
    /// The box the rect tool goes round: its corners' least and most, in
    /// the handles' frame, relative to `origin`. No box, no rect.
    pub bounds: Option<(Vec3, Vec3)>,
}

/// How the view sees the gizmo, in the handles' frame.
#[derive(Debug, Clone, Copy)]
struct Seen {
    origin: Vec3,
    eye: Vec3,
    /// From the eye toward the gizmo (along the view, when orthographic).
    toward: Vec3,
    right: Vec3,
    up: Vec3,
    length: f32,
    line: f32,
    slack: f32,
}

impl Seen {
    /// How far a point is from the eye, for drawing far things first.
    fn depth(&self, p: Vec3) -> f32 {
        (p - self.eye).dot(self.toward)
    }
}

/// The order parts are drawn in, before their depth: what is under what.
const BACK: u8 = 0;
const SWEEP: u8 = 1;
const RINGS: u8 = 2;
const HANDLES: u8 = 3;
const FRONT: u8 = 4;

/// Which grab wins where shapes overlap, first first: the middle, the
/// plane squares, the cubes and corners, the arrows and edges, the rings,
/// the ball and a rectangle's inside.
const RANK_CENTER: u8 = 0;
const RANK_PLANE: u8 = 1;
const RANK_KNOB: u8 = 2;
const RANK_AXIS: u8 = 3;
const RANK_RING: u8 = 4;
const RANK_AREA: u8 = 5;

/// One handle: what to draw and where it is grabbed.
#[derive(Debug, Clone)]
struct Part {
    grip: Grip,
    rank: u8,
    layer: u8,
    depth: f32,
    /// How opaque the whole of it is: an arm pointing at the eye fades.
    fade: f32,
    grabbable: bool,
    pieces: Vec<Paint>,
    shapes: Vec<Shape>,
}

impl Part {
    fn new(grip: Grip, rank: u8, layer: u8, depth: f32) -> Self {
        Self {
            grip,
            rank,
            layer,
            depth,
            fade: 1.0,
            grabbable: true,
            pieces: Vec::new(),
            shapes: Vec::new(),
        }
    }
}

/// A piece of a handle in the frame.
#[derive(Debug, Clone, Copy)]
enum Piece {
    Line { a: Vec3, b: Vec3, width: f32 },
    Cone { base: Vec3, tip: Vec3 },
    Cube { centre: Vec3, size: f32 },
    Triangle { a: Vec3, b: Vec3, c: Vec3 },
}

/// A piece and how it is painted: its handle's colour (yellow when hot)
/// unless it has its own, times its alpha.
#[derive(Debug, Clone, Copy)]
struct Paint {
    piece: Piece,
    color: Option<[f32; 3]>,
    alpha: f32,
    shaded: bool,
}

impl Paint {
    fn of(piece: Piece) -> Self {
        Self {
            piece,
            color: None,
            alpha: 1.0,
            shaded: false,
        }
    }
    fn alpha(mut self, alpha: f32) -> Self {
        self.alpha = alpha;
        self
    }
    fn color(mut self, color: [f32; 3]) -> Self {
        self.color = Some(color);
        self
    }
    fn shaded(mut self) -> Self {
        self.shaded = true;
        self
    }
}

/// Where a handle is grabbed, in the frame.
#[derive(Debug, Clone, Copy)]
enum Shape {
    /// Within `slack` of a line.
    Segment { a: Vec3, b: Vec3, slack: f32 },
    /// Inside a rectangle, `corner` plus up to `u` and `v`.
    Quad { corner: Vec3, u: Vec3, v: Vec3 },
    /// Within `radius` of a point.
    Ball { centre: Vec3, radius: f32 },
    /// Inside a circle.
    Disc { centre: Vec3, normal: Vec3, radius: f32 },
}

impl Shape {
    /// How squarely a ray hits, 0 dead on to 1 at the edge; `None` for a
    /// miss. `direction` is a unit vector.
    fn hit(&self, origin: Vec3, direction: Vec3) -> Option<f32> {
        match *self {
            Shape::Segment { a, b, slack } => {
                let (distance, _) = ray_to_segment(a, b, origin, direction)?;
                (distance <= slack).then(|| distance / slack.max(1e-9))
            }
            Shape::Quad { corner, u, v } => {
                let p = plane_hit(origin, direction, corner, u.cross(v))?;
                let at = p - corner;
                let s = at.dot(u) / u.length_squared().max(1e-12);
                let t = at.dot(v) / v.length_squared().max(1e-12);
                ((0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&t)).then_some(0.0)
            }
            Shape::Ball { centre, radius } => {
                let along = (centre - origin).dot(direction);
                if along < 0.0 {
                    return None;
                }
                let distance = (origin + direction * along - centre).length();
                (distance <= radius).then(|| distance / radius.max(1e-9))
            }
            Shape::Disc {
                centre,
                normal,
                radius,
            } => {
                let p = plane_hit(origin, direction, centre, normal)?;
                ((p - centre).length() <= radius).then_some(0.5)
            }
        }
    }
}

/// How opaque an arm is that points this nearly at the eye: whole until
/// about 15° off the line of sight, gone by about 6°. Unity fades them the
/// same way: an arm seen end-on is a dot, and a drag along it has no
/// answer.
fn axis_fade(axis: Vec3, toward: Vec3) -> f32 {
    let c = axis.dot(toward).abs();
    ((0.995 - c) / (0.995 - 0.965)).clamp(0.0, 1.0)
}

/// The same for a square seen edge-on.
fn plane_fade(normal: Vec3, toward: Vec3) -> f32 {
    let c = normal.dot(toward).abs();
    ((c - 0.05) / (0.2 - 0.05)).clamp(0.0, 1.0)
}

/// Points round a circle, `segments` of them.
fn circle(centre: Vec3, u: Vec3, v: Vec3, radius: f32, segments: usize) -> Vec<Vec3> {
    (0..=segments)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / segments as f32;
            centre + (u * a.cos() + v * a.sin()) * radius
        })
        .collect()
}

/// A filled disc, or a slice of one: triangles from the centre.
fn fan(centre: Vec3, u: Vec3, v: Vec3, radius: f32, from: f32, sweep: f32) -> Vec<Piece> {
    let steps = ((sweep.abs() / (std::f32::consts::TAU / 48.0)).ceil() as usize).max(1);
    let at = |i: usize| {
        let a = from + sweep * i as f32 / steps as f32;
        centre + (u * a.cos() + v * a.sin()) * radius
    };
    (0..steps)
        .map(|i| Piece::Triangle {
            a: centre,
            b: at(i),
            c: at(i + 1),
        })
        .collect()
}

/// Lines between consecutive points.
fn polyline(points: &[Vec3], width: f32) -> impl Iterator<Item = Piece> + '_ {
    points.windows(2).map(move |w| Piece::Line {
        a: w[0],
        b: w[1],
        width,
    })
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
        _ => (Vec3::X, Vec3::Y),
    }
}

/// The other two axes of one, in the order (u, v) with u × v along it.
fn others(axis: usize) -> (usize, usize) {
    ((axis + 1) % 3, (axis + 2) % 3)
}

impl Gizmo {
    /// A gizmo on the world's axes; [`Gizmo::turned`] for an entity's own.
    pub fn new(tool: Tool, camera: &Camera, style: &GizmoStyle, origin: Vec3) -> Self {
        Self {
            tool,
            camera: *camera,
            style: *style,
            origin,
            orientation: Quat::IDENTITY,
            bounds: None,
        }
    }

    /// The handles along the axes of a frame turned by `orientation`.
    pub fn turned(mut self, orientation: Quat) -> Self {
        self.orientation = orientation;
        self
    }

    /// The box the rect tool goes round, in the handles' frame, relative
    /// to the origin.
    pub fn around(mut self, min: Vec3, max: Vec3) -> Self {
        self.bounds = Some((min.min(max), min.max(max)));
        self
    }

    fn seen(&self) -> Seen {
        let back = self.orientation.inverse();
        let eye = self.origin + back * (self.camera.position - self.origin);
        let toward = match self.camera.ortho {
            Some(_) => back * (self.camera.target - self.camera.position),
            None => self.origin - eye,
        }
        .normalize_or(Vec3::NEG_Z);
        let up = back * self.camera.up;
        let right = toward
            .cross(up)
            .try_normalize()
            .unwrap_or_else(|| toward.any_orthonormal_vector());
        let up = right.cross(toward);
        let length = self.style.arm_length(&self.camera, self.origin);
        Seen {
            origin: self.origin,
            eye,
            toward,
            right,
            up,
            length,
            line: length * self.style.thickness,
            slack: length * self.style.grab_slack,
        }
    }

    /// Every handle this tool shows, in the frame.
    fn parts(&self, shown: &Shown) -> Vec<Part> {
        let s = self.seen();
        let l = s.length;
        let mut out = Vec::new();
        match self.tool {
            Tool::Move => {
                arrows(&s, 0.76 * l, 1.0 * l, &mut out);
                planes(&s, &mut out);
                middle_square(&s, &mut out);
            }
            Tool::Rotate => {
                ball(&s, l, true, &mut out);
                rings(&s, l, 1.1 * l, &mut out);
            }
            Tool::Scale => {
                scale_arms(&s, shown.stretch, &mut out);
            }
            Tool::Transform => {
                // Rings inside, arrows reaching past them with a cube
                // where the arrowhead begins, as Unity lays them out.
                ball(&s, 0.8 * l, false, &mut out);
                rings(&s, 0.8 * l, 0.88 * l, &mut out);
                arrows(&s, 1.1 * l, 1.34 * l, &mut out);
                for (i, axis) in Vec3::AXES.into_iter().enumerate() {
                    let fade = axis_fade(axis, s.toward);
                    let centre = s.origin + axis * 0.98 * l;
                    let size = 0.1 * l;
                    let mut part = Part::new(
                        Grip::new(Tool::Scale, Handle::along(i)),
                        RANK_KNOB,
                        HANDLES,
                        s.depth(centre),
                    );
                    part.fade = fade;
                    part.grabbable = fade > 0.5;
                    part.pieces
                        .push(Paint::of(Piece::Cube { centre, size }).shaded());
                    part.shapes.push(Shape::Ball {
                        centre,
                        radius: size * 0.9,
                    });
                    out.push(part);
                }
                planes(&s, &mut out);
                middle_square(&s, &mut out);
            }
            Tool::Rect => {
                if let Some((min, max)) = self.bounds {
                    rect(&s, min, max, &mut out);
                }
            }
        }
        if let Some(swept) = shown.swept {
            out.push(wedge(&s, swept));
        }
        out
    }

    /// The draw list, in the world, far parts first and the middle last.
    pub fn draws(&self, meshes: &GizmoMeshes, shown: &Shown) -> Vec<Draw> {
        let mut parts = self.parts(shown);
        parts.sort_by(|a, b| a.layer.cmp(&b.layer).then(b.depth.total_cmp(&a.depth)));
        let turn = Mat4::from_translation(self.origin)
            * Mat4::from_quat(self.orientation)
            * Mat4::from_translation(-self.origin);
        let hot = hot_color().base_color;
        let mut out = Vec::new();
        for part in parts {
            let lit = match shown.held {
                Some(held) => held == part.grip,
                None => shown.hovered == Some(part.grip),
            };
            let base = if lit {
                hot
            } else {
                part.grip.handle.color().base_color
            };
            for paint in part.pieces {
                let alpha = (paint.alpha * part.fade).clamp(0.0, 1.0);
                if alpha < 0.01 {
                    continue;
                }
                // A piece with its own colour keeps it: the ball's rim, a
                // pie in its ring's colour, a dot's white edge.
                let color = paint.color.unwrap_or(base);
                let (mesh, transform) = placed(meshes, paint.piece);
                out.push(Draw {
                    mesh,
                    transform: turn * transform,
                    texture: TextureHandle::WHITE,
                    material: painted(color, alpha, paint.shaded),
                    pose: None,
                });
            }
        }
        out
    }

    /// Which handle a ray grabs, if any: the first by rank (the middle,
    /// squares, cubes, arrows, rings, the ball), and of those the one it
    /// passes nearest.
    pub fn pick(&self, ray_origin: Vec3, ray_direction: Vec3) -> Option<Grip> {
        let (o, d) = ray_into(self.origin, self.orientation, ray_origin, ray_direction);
        let d = d.normalize_or_zero();
        let mut best: Option<(u8, f32, Grip)> = None;
        for part in self.parts(&Shown::default()) {
            if !part.grabbable {
                continue;
            }
            for shape in &part.shapes {
                let Some(score) = shape.hit(o, d) else {
                    continue;
                };
                if best.is_none_or(|(rank, s, _)| (part.rank, score) < (rank, s)) {
                    best = Some((part.rank, score, part.grip));
                }
            }
        }
        best.map(|(_, _, grip)| grip)
    }

    /// Start a drag on a handle, where a ray grabbed it.
    pub fn begin(&self, grip: Grip, ray_origin: Vec3, ray_direction: Vec3) -> Drag {
        let s = self.seen();
        let (o, d) = ray_into(self.origin, self.orientation, ray_origin, ray_direction);
        let d = d.normalize_or_zero();
        let mut drag = Drag {
            tool: grip.tool,
            handle: grip.handle,
            origin: self.origin,
            orientation: self.orientation,
            grab_offset: 0.0,
            grab_point: self.origin,
            toward: s.toward,
            right: s.right,
            up: s.up,
            length: s.length,
            tangent: None,
            rect: None,
        };
        let origin = self.origin;
        match (grip.tool, grip.handle) {
            (Tool::Rotate, Handle::X | Handle::Y | Handle::Z) => {
                let axis = grip.handle.axis();
                if axis.dot(s.toward).abs() < 0.25 {
                    // Nearly edge-on: the ring's point nearest the ray,
                    // and the tangent there.
                    let radius = match self.tool {
                        Tool::Transform => 0.8 * s.length,
                        _ => s.length,
                    };
                    let (u, v) = ring_plane(grip.handle);
                    let nearest = circle(origin, u, v, radius, 256)
                        .into_iter()
                        .min_by(|a, b| {
                            ray_distance(*a, o, d).total_cmp(&ray_distance(*b, o, d))
                        })
                        .unwrap_or(origin + u * radius);
                    drag.grab_point = nearest;
                    drag.tangent = axis.cross(nearest - origin).try_normalize();
                } else {
                    drag.grab_point = plane_hit_line(o, d, origin, axis).unwrap_or(origin);
                    drag.grab_offset = angle_at(origin, grip.handle, o, d).unwrap_or(0.0);
                }
            }
            (Tool::Rotate, Handle::View) => {
                drag.grab_point = plane_hit_line(o, d, origin, s.toward).unwrap_or(origin);
            }
            (Tool::Rotate, _) => {
                drag.grab_point = origin + drag.ball_point(o, d).unwrap_or(Vec3::ZERO);
            }
            (Tool::Rect, Handle::Rect(sx, sy)) => {
                if let Some((min, max)) = self.bounds {
                    let (across, u, v) = rect_axes(&s, min, max);
                    let plane = origin + Vec3::AXES[across] * ((min + max) * 0.5)[across];
                    let mut anchor = origin;
                    for (axis, side) in [(u, sx), (v, sy)] {
                        // The far side stays put.
                        anchor[axis] = origin[axis]
                            + match side {
                                1 => min[axis],
                                -1 => max[axis],
                                _ => 0.0,
                            };
                    }
                    drag.rect = Some(RectGrab {
                        across,
                        u,
                        v,
                        plane,
                        anchor,
                    });
                    drag.grab_point =
                        plane_hit_line(o, d, plane, Vec3::AXES[across]).unwrap_or(origin);
                }
            }
            (_, Handle::X | Handle::Y | Handle::Z) => {
                drag.grab_offset = closest_points(origin, grip.handle.axis(), o, d)
                    .map(|(along, _)| along)
                    // A ray exactly along the axis has no unique closest
                    // point. Treating the grab as being at the origin is
                    // wrong by less than the width of the handle, and the
                    // alternative is refusing to start a drag the user
                    // clearly asked for.
                    .unwrap_or(0.0);
            }
            (_, Handle::PlaneX | Handle::PlaneY | Handle::PlaneZ) => {
                drag.grab_point =
                    plane_hit_line(o, d, origin, grip.handle.axis()).unwrap_or(origin);
            }
            _ => {
                drag.grab_point = plane_hit_line(o, d, origin, s.toward).unwrap_or(origin);
            }
        }
        drag
    }
}

/// A Draw's mesh and transform for a piece, in the frame.
fn placed(meshes: &GizmoMeshes, piece: Piece) -> (MeshHandle, Mat4) {
    match piece {
        Piece::Line { a, b, width } => {
            let along = b - a;
            let length = along.length().max(1e-6);
            (
                meshes.cube,
                Mat4::from_scale_rotation_translation(
                    // Longer by its width, so lines meeting at an angle
                    // (round a ring) leave no notch.
                    Vec3::new(length + width, width, width),
                    Quat::from_rotation_arc(Vec3::X, along / length),
                    (a + b) * 0.5,
                ),
            )
        }
        Piece::Cone { base, tip } => {
            let along = tip - base;
            let length = along.length().max(1e-6);
            (
                meshes.cone,
                Mat4::from_scale_rotation_translation(
                    Vec3::splat(length),
                    Quat::from_rotation_arc(Vec3::Y, along / length),
                    base,
                ),
            )
        }
        Piece::Cube { centre, size } => (
            meshes.cube,
            Mat4::from_scale_rotation_translation(Vec3::splat(size), Quat::IDENTITY, centre),
        ),
        Piece::Triangle { a, b, c } => {
            let normal = (b - a).cross(c - a).normalize_or(Vec3::Z);
            (
                meshes.triangle,
                Mat4::from_cols(
                    (b - a).extend(0.0),
                    (c - a).extend(0.0),
                    normal.extend(0.0),
                    a.extend(1.0),
                ),
            )
        }
    }
}

/// A handle's material: its colour, unlit or shaded (for a tool, by a lamp
/// at the eye), see-through below one.
fn painted(color: [f32; 3], alpha: f32, shaded: bool) -> Material {
    let mut m = Material::new(color[0], color[1], color[2]);
    m.shading = if shaded { Shading::Lit } else { Shading::Unlit };
    if alpha < 1.0 {
        m.alpha = alpha;
        m.surface = SurfaceType::Transparent;
    }
    m
}

/// Move's arrows: a shaft from the middle to the head, a shaded cone from
/// `head` to `tip` along the arm.
fn arrows(s: &Seen, head: f32, tip: f32, out: &mut Vec<Part>) {
    for (i, axis) in Vec3::AXES.into_iter().enumerate() {
        let fade = axis_fade(axis, s.toward);
        let (base, end) = (s.origin + axis * head, s.origin + axis * tip);
        let mut part = Part::new(
            Grip::new(Tool::Move, Handle::along(i)),
            RANK_AXIS,
            HANDLES,
            s.depth(s.origin + axis * tip * 0.5),
        );
        part.fade = fade;
        part.grabbable = fade > 0.5;
        part.pieces.push(Paint::of(Piece::Line {
            a: s.origin,
            b: base,
            width: s.line,
        }));
        part.pieces
            .push(Paint::of(Piece::Cone { base, tip: end }).shaded());
        // Not from the very middle: that is the middle square's.
        part.shapes.push(Shape::Segment {
            a: s.origin + axis * 0.1 * s.length,
            b: end,
            slack: s.slack.max((tip - head) * CONE_ASPECT * 1.2),
        });
        out.push(part);
    }
}

/// Move's squares between the axes, each in the quarter of its plane that
/// faces the eye — they flip over as the view goes round, as Unity's do.
fn planes(s: &Seen, out: &mut Vec<Part>) {
    let (near, far) = (0.1 * s.length, 0.32 * s.length);
    let to_eye = -s.toward;
    for across in 0..3 {
        let normal = Vec3::AXES[across];
        let (u, v) = others(across);
        let side = |i: usize| {
            if to_eye[i] >= 0.0 {
                Vec3::AXES[i]
            } else {
                -Vec3::AXES[i]
            }
        };
        let (du, dv) = (side(u), side(v));
        let corner = s.origin + (du + dv) * near;
        let (eu, ev) = (du * (far - near), dv * (far - near));
        let fade = plane_fade(normal, s.toward);
        let mut part = Part::new(
            Grip::new(Tool::Move, Handle::across(across)),
            RANK_PLANE,
            HANDLES,
            s.depth(corner + (eu + ev) * 0.5),
        );
        part.fade = fade;
        part.grabbable = fade > 0.5;
        let quad = [corner, corner + eu, corner + eu + ev, corner + ev];
        for (a, b, c) in [(0, 1, 2), (0, 2, 3)] {
            part.pieces.push(
                Paint::of(Piece::Triangle {
                    a: quad[a],
                    b: quad[b],
                    c: quad[c],
                })
                .alpha(0.3),
            );
        }
        let mut edge = quad.to_vec();
        edge.push(quad[0]);
        part.pieces
            .extend(polyline(&edge, s.line * 0.8).map(|p| Paint::of(p).alpha(0.9)));
        part.shapes.push(Shape::Quad { corner, u: eu, v: ev });
        out.push(part);
    }
}

/// Move's middle: a small square facing the eye, for a drag across the
/// view's plane.
fn middle_square(s: &Seen, out: &mut Vec<Part>) {
    let half = 0.07 * s.length;
    let (r, u) = (s.right * half, s.up * half);
    let quad = [
        s.origin - r - u,
        s.origin + r - u,
        s.origin + r + u,
        s.origin - r + u,
    ];
    let mut part = Part::new(
        Grip::new(Tool::Move, Handle::Center),
        RANK_CENTER,
        FRONT,
        s.depth(s.origin),
    );
    for (a, b, c) in [(0, 1, 2), (0, 2, 3)] {
        part.pieces.push(
            Paint::of(Piece::Triangle {
                a: quad[a],
                b: quad[b],
                c: quad[c],
            })
            .alpha(0.2),
        );
    }
    let mut edge = quad.to_vec();
    edge.push(quad[0]);
    part.pieces
        .extend(polyline(&edge, s.line * 0.8).map(Paint::of));
    // A little larger than drawn: it is small.
    let grab = 1.25;
    part.shapes.push(Shape::Quad {
        corner: s.origin - (r + u) * grab,
        u: r * 2.0 * grab,
        v: u * 2.0 * grab,
    });
    out.push(part);
}

/// Scale's arms, each stretched by its axis's factor while a drag lasts,
/// with a shaded cube at the end, and a cube in the middle for all at once.
fn scale_arms(s: &Seen, stretch: Vec3, out: &mut Vec<Part>) {
    let size = 0.1 * s.length;
    for (i, axis) in Vec3::AXES.into_iter().enumerate() {
        let reach = 0.9 * s.length * stretch[i].clamp(0.05, 8.0);
        let end = s.origin + axis * reach;
        let fade = axis_fade(axis, s.toward);
        let mut part = Part::new(
            Grip::new(Tool::Scale, Handle::along(i)),
            RANK_AXIS,
            HANDLES,
            s.depth(s.origin + axis * reach * 0.5),
        );
        part.fade = fade;
        part.grabbable = fade > 0.5;
        part.pieces.push(Paint::of(Piece::Line {
            a: s.origin,
            b: end,
            width: s.line,
        }));
        part.pieces
            .push(Paint::of(Piece::Cube { centre: end, size }).shaded());
        part.shapes.push(Shape::Segment {
            a: s.origin + axis * 0.12 * s.length,
            b: end,
            slack: s.slack.max(size * 0.7),
        });
        out.push(part);
    }
    let mut middle = Part::new(
        Grip::new(Tool::Scale, Handle::Center),
        RANK_CENTER,
        FRONT,
        s.depth(s.origin),
    );
    let size = 0.13 * s.length;
    middle.pieces.push(
        Paint::of(Piece::Cube {
            centre: s.origin,
            size,
        })
        .shaded(),
    );
    middle.shapes.push(Shape::Ball {
        centre: s.origin,
        radius: size * 0.8,
    });
    out.push(middle);
}

/// Rotate's ball: a faint disc facing the eye, the trackball, and its
/// rim in grey. Only the rim for the Transform tool, where the inside is
/// the arrows'.
fn ball(s: &Seen, radius: f32, grabbable: bool, out: &mut Vec<Part>) {
    let mut part = Part::new(
        Grip::new(Tool::Rotate, Handle::Free),
        RANK_AREA,
        BACK,
        s.depth(s.origin),
    );
    part.grabbable = grabbable;
    if grabbable {
        part.pieces.extend(
            fan(s.origin, s.right, s.up, radius, 0.0, std::f32::consts::TAU)
                .into_iter()
                .map(|p| Paint::of(p).alpha(0.08)),
        );
        part.shapes.push(Shape::Disc {
            centre: s.origin,
            normal: s.toward,
            radius,
        });
    }
    let rim = circle(s.origin, s.right, s.up, radius, 96);
    let grey = Material::from_srgb(128, 128, 128).base_color;
    part.pieces.extend(
        polyline(&rim, s.line * 0.6).map(|p| Paint::of(p).color(grey).alpha(0.6)),
    );
    out.push(part);
}

/// Rotate's rings: one about each axis, only the half facing the eye —
/// the back half would cross the front one and nobody can tell which is
/// which — and a bigger one about the line of sight, whole.
fn rings(s: &Seen, radius: f32, outer: f32, out: &mut Vec<Part>) {
    let to_eye = -s.toward;
    const SEGMENTS: usize = 96;
    for (i, handle) in Handle::ALL.into_iter().enumerate() {
        let (u, v) = ring_plane(handle);
        let points = circle(s.origin, u, v, radius, SEGMENTS);
        let mut part = Part::new(
            Grip::new(Tool::Rotate, handle),
            RANK_RING,
            RINGS,
            s.depth(s.origin) - i as f32 * 1e-4,
        );
        for w in points.windows(2) {
            let middle = (w[0] + w[1]) * 0.5 - s.origin;
            // The near half: in front of the plane through the middle,
            // across the line of sight. A ring seen face-on is all near.
            if middle.dot(to_eye) < -1e-3 * radius {
                continue;
            }
            part.pieces.push(Paint::of(Piece::Line {
                a: w[0],
                b: w[1],
                width: s.line * 1.2,
            }));
            part.shapes.push(Shape::Segment {
                a: w[0],
                b: w[1],
                slack: s.slack,
            });
        }
        out.push(part);
    }
    let points = circle(s.origin, s.right, s.up, outer, SEGMENTS);
    let mut view = Part::new(
        Grip::new(Tool::Rotate, Handle::View),
        RANK_RING,
        RINGS,
        s.depth(s.origin),
    );
    for w in points.windows(2) {
        view.pieces.push(Paint::of(Piece::Line {
            a: w[0],
            b: w[1],
            width: s.line,
        }));
        view.shapes.push(Shape::Segment {
            a: w[0],
            b: w[1],
            slack: s.slack,
        });
    }
    out.push(view);
}

/// The pie a turn has swept, in its ring's colour, and its two edges.
fn wedge(s: &Seen, swept: Swept) -> Part {
    let mut part = Part::new(
        Grip::new(Tool::Rotate, swept.handle),
        u8::MAX,
        SWEEP,
        s.depth(s.origin),
    );
    part.grabbable = false;
    let u = swept.from.normalize_or(Vec3::X);
    let v = swept.axis.normalize_or(Vec3::Y).cross(u);
    let color = swept.handle.color().base_color;
    part.pieces.extend(
        fan(s.origin, u, v, swept.radius, 0.0, swept.angle)
            .into_iter()
            .map(|p| Paint::of(p).color(color).alpha(0.3)),
    );
    let end = s.origin + (u * swept.angle.cos() + v * swept.angle.sin()) * swept.radius;
    for b in [s.origin + u * swept.radius, end] {
        part.pieces.push(
            Paint::of(Piece::Line {
                a: s.origin,
                b,
                width: s.line * 0.7,
            })
            .color(color)
            .alpha(0.8),
        );
    }
    part
}

/// The rect's plane: the axis across it (the one pointing most at the
/// eye, of those the box is not flat along the other two of) and the two
/// it spans.
fn rect_axes(s: &Seen, min: Vec3, max: Vec3) -> (usize, usize, usize) {
    let size = max - min;
    let mut order = [0usize, 1, 2];
    order.sort_by(|a, b| s.toward[*b].abs().total_cmp(&s.toward[*a].abs()));
    let across = order
        .into_iter()
        .find(|&k| {
            let (u, v) = others(k);
            size[u] > 1e-5 && size[v] > 1e-5
        })
        .unwrap_or(order[0]);
    let (u, v) = others(across);
    (across, u, v)
}

/// The rect tool: the rectangle's outline, a dot on each corner and edge,
/// the pivot's circle, the inside to move by.
fn rect(s: &Seen, min: Vec3, max: Vec3, out: &mut Vec<Part>) {
    let (across, u, v) = rect_axes(s, min, max);
    let depth = ((min + max) * 0.5)[across];
    let at = |a: f32, b: f32| {
        let mut p = Vec3::ZERO;
        p[across] = depth;
        p[u] = a;
        p[v] = b;
        s.origin + p
    };
    let side = |axis: usize, k: i8| match k {
        -1 => min[axis],
        1 => max[axis],
        _ => (min[axis] + max[axis]) * 0.5,
    };
    let corner = at(min[u], min[v]);
    let (eu, ev) = (
        at(max[u], min[v]) - corner,
        at(min[u], max[v]) - corner,
    );
    let mut inside = Part::new(
        Grip::new(Tool::Rect, Handle::Rect(0, 0)),
        RANK_AREA,
        BACK,
        s.depth(corner),
    );
    let quad = [corner, corner + eu, corner + eu + ev, corner + ev, corner];
    inside
        .pieces
        .extend(polyline(&quad, s.line * 0.8).map(|p| Paint::of(p).alpha(0.9)));
    let mut pivot = s.origin;
    pivot[across] = s.origin[across] + depth;
    let dial = circle(pivot, s.right, s.up, 0.045 * s.length, 32);
    inside
        .pieces
        .extend(polyline(&dial, s.line * 0.7).map(Paint::of));
    inside.shapes.push(Shape::Quad {
        corner,
        u: eu,
        v: ev,
    });
    out.push(inside);
    let white = Material::from_srgb(255, 255, 255).base_color;
    for sx in -1i8..=1 {
        for sy in -1i8..=1 {
            if sx == 0 && sy == 0 {
                continue;
            }
            let p = at(side(u, sx), side(v, sy));
            let dot = if sx != 0 && sy != 0 { 0.035 } else { 0.028 } * s.length;
            let mut part = Part::new(
                Grip::new(Tool::Rect, Handle::Rect(sx, sy)),
                if sx != 0 && sy != 0 { RANK_KNOB } else { RANK_AXIS },
                FRONT,
                s.depth(p),
            );
            // A white rim round a blue dot.
            part.pieces.extend(
                fan(p, s.right, s.up, dot + 0.01 * s.length, 0.0, std::f32::consts::TAU)
                    .into_iter()
                    .map(|t| Paint::of(t).color(white)),
            );
            part.pieces.extend(
                fan(p, s.right, s.up, dot, 0.0, std::f32::consts::TAU)
                    .into_iter()
                    .map(Paint::of),
            );
            part.shapes.push(Shape::Ball {
                centre: p,
                radius: dot * 1.6,
            });
            if sx == 0 || sy == 0 {
                // An edge is grabbed along its length, not just at its dot.
                let (a, b) = if sx == 0 {
                    (at(min[u], side(v, sy)), at(max[u], side(v, sy)))
                } else {
                    (at(side(u, sx), min[v]), at(side(u, sx), max[v]))
                };
                part.shapes.push(Shape::Segment {
                    a,
                    b,
                    slack: s.slack * 0.8,
                });
            }
            out.push(part);
        }
    }
}

impl Drag {
    /// What the drag asks for now, given where the cursor's ray is, in the
    /// world.
    pub fn motion(&self, ray_origin: Vec3, ray_direction: Vec3) -> Motion {
        self.motion_snapped(ray_origin, ray_direction, 0.0)
    }

    /// The same, a turn about one ring in steps of `degrees` (none at 0).
    pub fn motion_snapped(&self, ray_origin: Vec3, ray_direction: Vec3, degrees: f32) -> Motion {
        let (o, d) = ray_into(self.origin, self.orientation, ray_origin, ray_direction);
        let solved = self.solve(o, d.normalize_or_zero(), degrees);
        motion_out_of(solved, self.origin, self.orientation)
    }

    /// How far a ring's drag has turned, in radians, before snapping; `None`
    /// for anything but a ring, or when the ray gives no answer.
    pub fn angle(&self, ray_origin: Vec3, ray_direction: Vec3) -> Option<f32> {
        let (o, d) = ray_into(self.origin, self.orientation, ray_origin, ray_direction);
        self.turn(o, d.normalize_or_zero()).map(|(_, angle)| angle)
    }

    /// The pie to draw for a turn of `angle` radians, in the frame.
    pub fn swept(&self, angle: f32) -> Option<Swept> {
        let axis = self.turn_axis()?;
        let from = self.grab_point - self.origin;
        let from = from - axis * from.dot(axis);
        Some(Swept {
            handle: self.handle,
            axis,
            from: from.try_normalize()?,
            angle,
            radius: from.length(),
        })
    }

    /// The axis a ring turns about, in the frame.
    fn turn_axis(&self) -> Option<Vec3> {
        match (self.tool, self.handle) {
            (Tool::Rotate, Handle::X | Handle::Y | Handle::Z) => Some(self.handle.axis()),
            (Tool::Rotate, Handle::View) => Some(self.toward),
            _ => None,
        }
    }

    /// A ring's turn for a ray in the frame: about what, and how far.
    fn turn(&self, o: Vec3, d: Vec3) -> Option<(Vec3, f32)> {
        let axis = self.turn_axis()?;
        if let Some(tangent) = self.tangent {
            // Along the tangent, a ring's circumference to a turn.
            let (along, _) = closest_points(self.grab_point, tangent, o, d)?;
            let radius = (self.grab_point - self.origin).length().max(1e-6);
            return Some((axis, along / radius));
        }
        let (angle, grabbed) = match self.handle {
            Handle::View => {
                let u = (self.grab_point - self.origin).try_normalize()?;
                let v = axis.cross(u);
                let p = plane_hit_line(o, d, self.origin, axis)? - self.origin;
                (p.dot(v).atan2(p.dot(u)), 0.0)
            }
            _ => (angle_at(self.origin, self.handle, o, d)?, self.grab_offset),
        };
        // Wrapped into a half turn either way, so that dragging past the
        // far side of the ring keeps turning the same direction instead
        // of snapping most of the way round.
        let mut delta = angle - grabbed;
        while delta > std::f32::consts::PI {
            delta -= std::f32::consts::TAU;
        }
        while delta < -std::f32::consts::PI {
            delta += std::f32::consts::TAU;
        }
        Some((axis, delta))
    }

    /// Where a ray meets the ball, relative to its middle: on its near
    /// side inside the rim, on the rim outside it.
    fn ball_point(&self, o: Vec3, d: Vec3) -> Option<Vec3> {
        let off = plane_hit_line(o, d, self.origin, self.toward)? - self.origin;
        let (r, reach) = (self.length, off.length());
        Some(if reach >= r {
            off / reach.max(1e-9) * r
        } else {
            off - self.toward * (r * r - reach * reach).sqrt()
        })
    }

    /// The drag's answer in the frame.
    fn solve(&self, o: Vec3, d: Vec3, degrees: f32) -> Motion {
        let origin = self.origin;
        let stay = match self.tool {
            Tool::Rotate => Motion::Rotation(Quat::IDENTITY),
            Tool::Scale => Motion::Scale(Vec3::ONE),
            _ => Motion::Position(origin),
        };
        // No answer — edge-on, parallel, behind the eye — means staying
        // put: moving by a guess is how a gizmo flings an object off
        // screen.
        let slide = |normal: Vec3, through: Vec3| {
            plane_hit_line(o, d, through, normal).map(|p| {
                // Exactly in the plane: what is across it stays to the bit.
                let step = p - self.grab_point;
                Motion::Position(origin + step - normal * step.dot(normal))
            })
        };
        let answer = match (self.tool, self.handle) {
            (Tool::Move, Handle::X | Handle::Y | Handle::Z) => {
                let axis = self.handle.axis();
                closest_points(origin, axis, o, d)
                    .map(|(along, _)| Motion::Position(origin + axis * (along - self.grab_offset)))
            }
            (Tool::Move, Handle::PlaneX | Handle::PlaneY | Handle::PlaneZ) => {
                slide(self.handle.axis(), origin)
            }
            (Tool::Move, _) => slide(self.toward, origin),
            (Tool::Rotate, Handle::Free) => {
                let from = (self.grab_point - origin).try_normalize();
                let to = self.ball_point(o, d).and_then(|p| p.try_normalize());
                from.zip(to)
                    .map(|(from, to)| Motion::Rotation(Quat::from_rotation_arc(from, to)))
            }
            (Tool::Rotate, _) => self.turn(o, d).map(|(axis, angle)| {
                let angle = snap(angle, degrees.to_radians());
                Motion::Rotation(Quat::from_axis_angle(axis, angle))
            }),
            (Tool::Scale, Handle::Center) => plane_hit_line(o, d, origin, self.toward).map(|p| {
                // Right or up grows it, left or down shrinks it, a whole
                // arm's length doubling it.
                let moved = p - self.grab_point;
                let amount = (moved.dot(self.right) + moved.dot(self.up)) / self.length;
                Motion::Scale(Vec3::splat((1.0 + amount).max(0.01)))
            }),
            (Tool::Scale, _) => {
                let axis = self.handle.axis();
                closest_points(origin, axis, o, d).map(|(along, _)| {
                    // Grabbing at the origin gives nothing to divide by;
                    // so does dragging through it. Both are clamped rather
                    // than refused, so a drag that passes the pivot shrinks
                    // toward nothing instead of turning the object inside
                    // out.
                    let grabbed = self.grab_offset.abs().max(1e-3);
                    let factor = (along / grabbed).max(0.01);
                    Motion::Scale(Vec3::ONE + axis * (factor - 1.0))
                })
            }
            (Tool::Rect, Handle::Rect(sx, sy)) => self.rect.and_then(|r| {
                let normal = Vec3::AXES[r.across];
                if (sx, sy) == (0, 0) {
                    return slide(normal, r.plane);
                }
                let p = plane_hit_line(o, d, r.plane, normal)?;
                let mut scale = Vec3::ONE;
                for (axis, side) in [(r.u, sx), (r.v, sy)] {
                    let span = self.grab_point[axis] - r.anchor[axis];
                    if side != 0 && span.abs() > 1e-6 {
                        // Never through the far side: a rectangle turned
                        // inside out is an object turned inside out.
                        scale[axis] = ((p[axis] - r.anchor[axis]) / span).max(0.01);
                    }
                }
                Some(Motion::Resize {
                    scale,
                    about: r.anchor,
                })
            }),
            _ => None,
        };
        answer.unwrap_or(stay)
    }
}

/// Where the cursor's ray crosses a ring's plane, as an angle around it.
///
/// `None` when the ray runs inside the plane and never crosses it.
fn angle_at(origin: Vec3, handle: Handle, ray_origin: Vec3, ray_direction: Vec3) -> Option<f32> {
    let axis = handle.axis();
    let point = plane_hit_line(ray_origin, ray_direction, origin, axis)? - origin;
    let (u, v) = ring_plane(handle);
    let (x, y) = (point.dot(u), point.dot(v));
    if x.abs() < 1e-6 && y.abs() < 1e-6 {
        // Dead centre: no angle. Refusing beats spinning on a rounding
        // error at the pivot.
        return None;
    }
    Some(y.atan2(x))
}

/// Where a ray crosses a plane, in front of its origin; `None` when it
/// runs along it or away from it.
fn plane_hit(origin: Vec3, direction: Vec3, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let facing = normal.dot(direction);
    if facing.abs() < 1e-6 {
        return None;
    }
    let travel = normal.dot(point - origin) / facing;
    (travel >= 0.0).then(|| origin + direction * travel)
}

/// Where the line of a ray crosses a plane, behind or in front; `None`
/// when it runs along it (within a degree or so, where the crossing flies
/// off to somewhere nobody meant).
fn plane_hit_line(origin: Vec3, direction: Vec3, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let direction = direction.normalize_or_zero();
    let facing = normal.dot(direction);
    if facing.abs() < 1e-2 {
        return None;
    }
    Some(origin + direction * (normal.dot(point - origin) / facing))
}

/// How far a point is from a ray's line.
fn ray_distance(point: Vec3, origin: Vec3, direction: Vec3) -> f32 {
    let along = (point - origin).dot(direction);
    (origin + direction * along - point).length()
}

/// How near a ray (unit direction) passes a segment, and how far along
/// the ray; `None` when the nearest point is behind it.
fn ray_to_segment(a: Vec3, b: Vec3, origin: Vec3, direction: Vec3) -> Option<(f32, f32)> {
    let along = b - a;
    let length = along.length();
    let t = if length < 1e-9 {
        0.0
    } else {
        match closest_points(a, along / length, origin, direction) {
            Some((t, _)) => t.clamp(0.0, length),
            // Parallel: every point is as near; the nearer end will do.
            None => 0.0,
        }
    };
    let p = if length < 1e-9 { a } else { a + along / length * t };
    let ray_at = (p - origin).dot(direction);
    if ray_at < 0.0 {
        return None;
    }
    Some(((origin + direction * ray_at - p).length(), ray_at))
}
/// Walkable ground: Unity's navmesh blue.
pub fn navigation_color() -> Material {
    Material::new(0.25, 0.6, 1.0).unlit()
}

/// A camera's frustum: a quiet white, like Unity's.
pub fn camera_color() -> Material {
    Material::new(0.9, 0.9, 0.9).unlit()
}

/// Where the running game has a thing, outlined in the editor: cyan, not
/// to be taken for the selection.
pub fn game_color() -> Material {
    Material::new(0.1, 0.85, 1.0).unlit()
}

/// What is selected is outlined in this: Unity's orange.
pub fn selection_color() -> Material {
    Material::from_srgb(255, 102, 0).unlit()
}

/// What hangs under a selected thing is outlined in this: Unity's blue.
pub fn child_selection_color() -> Material {
    Material::from_srgb(94, 119, 255).unlit()
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
    // The box's own sizes scaled, then placed without scale, as a
    // collider's outline is: lines keep their thickness whatever the scale.
    let half = ((max - min) * 0.5).max(Vec3::splat(1e-4));
    let centre = placed * Mat4::from_translation((min + max) * 0.5);
    let (scale, rotation, translation) = centre.to_scale_rotation_translation();
    let h = half * scale;
    let corner = |x: f32, y: f32, z: f32| Vec3::new(x * h.x, y * h.y, z * h.z);
    let mut segments = Vec::with_capacity(12);
    for (a, b) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
        segments.push((corner(-1.0, a, b), corner(1.0, a, b)));
        segments.push((corner(a, -1.0, b), corner(a, 1.0, b)));
        segments.push((corner(a, b, -1.0), corner(a, b, 1.0)));
    }
    line_draws(
        arm,
        segments,
        Mat4::from_rotation_translation(rotation, translation),
        thickness,
        material,
    )
}

/// A camera as lines: where it stands and what it sees, `depth` metres out
/// — Unity's camera gizmo. Along the entity's +z, as the camera looks; a
/// box for an orthographic one. `aspect` is the view's width over height.
pub fn camera_draws(
    arm: MeshHandle,
    lens: crate::scene::Lens,
    placed: Mat4,
    aspect: f32,
    depth: f32,
    thickness: f32,
    material: Material,
) -> Vec<Draw> {
    let (_, rotation, translation) = placed.to_scale_rotation_translation();
    let frame = Mat4::from_rotation_translation(rotation, translation);
    let (near_half, far_half) = match lens.ortho {
        Some(half) => (half, half),
        None => (0.0, (lens.fov_deg.to_radians() * 0.5).tan() * depth),
    };
    let corners = |half: f32, z: f32| {
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(x, y)| Vec3::new(x * half * aspect, y * half, z))
    };
    let (near, far) = (corners(near_half, 0.0), corners(far_half, depth));
    let mut segments = Vec::with_capacity(12);
    for i in 0..4 {
        let j = (i + 1) % 4;
        segments.push((far[i], far[j]));
        segments.push((near[i], far[i]));
        if lens.ortho.is_some() {
            segments.push((near[i], near[j]));
        }
    }
    // Which way is up in the picture: a mark over the far edge.
    let top = Vec3::new(0.0, far_half * 1.3, depth);
    segments.push((far[2], top));
    segments.push((far[3], top));
    line_draws(arm, segments, frame, thickness, material)
}

/// A line through points, in world space, as thin boxes: a route drawn
/// in the Scene view.
pub fn polyline_draws(
    arm: MeshHandle,
    points: &[Vec3],
    thickness: f32,
    material: Material,
) -> Vec<Draw> {
    let segments = points.windows(2).map(|w| (w[0], w[1])).collect();
    line_draws(arm, segments, Mat4::IDENTITY, thickness, material)
}

/// A standing capsule as lines, feet at `feet`: the player's size in the
/// Scene view. Two rings where the rounded ends begin, four sides, the
/// ends' arcs across both ways, and a line from the chest along `facing`.
pub fn capsule_draws(
    arm: MeshHandle,
    feet: Vec3,
    height: f32,
    radius: f32,
    facing: Vec3,
    thickness: f32,
    material: Material,
) -> Vec<Draw> {
    const SIDES: usize = 24;
    let radius = radius.max(0.01);
    let (low, high) = (radius, (height - radius).max(radius));
    let turn = |i: usize| i as f32 / SIDES as f32 * std::f32::consts::TAU;
    let mut segments = Vec::new();
    for y in [low, high] {
        for i in 0..SIDES {
            let (a, b) = (turn(i), turn(i + 1));
            segments.push((
                feet + Vec3::new(a.cos() * radius, y, a.sin() * radius),
                feet + Vec3::new(b.cos() * radius, y, b.sin() * radius),
            ));
        }
    }
    for across in [Vec3::X, Vec3::Z] {
        for side in [-1.0, 1.0] {
            segments.push((
                feet + across * side * radius + Vec3::Y * low,
                feet + across * side * radius + Vec3::Y * high,
            ));
        }
        // Half a circle over the top and under the bottom.
        for i in 0..SIDES / 2 {
            let (a, b) = (turn(i), turn(i + 1));
            let at = |t: f32, y: f32, down: f32| {
                feet + across * (t.cos() * radius) + Vec3::Y * (y + down * t.sin() * radius)
            };
            segments.push((at(a, high, 1.0), at(b, high, 1.0)));
            segments.push((at(a, low, -1.0), at(b, low, -1.0)));
        }
    }
    let facing = Vec3::new(facing.x, 0.0, facing.z).normalize_or_zero();
    if facing != Vec3::ZERO {
        let chest = feet + Vec3::Y * (height * 0.75);
        segments.push((chest, chest + facing * radius * 2.0));
    }
    line_draws(arm, segments, Mat4::IDENTITY, thickness, material)
}

/// What the player reference and its jump are drawn in: a light green,
/// not to be taken for a camera's white or the selection's orange.
pub fn player_color() -> Material {
    Material::new(0.45, 0.95, 0.35).unlit()
}

/// The Scene view's grid: lines `spacing` apart on the plane through the
/// origin across `axis` (1: the ground; 0 and 2: the walls a side view
/// looks at), `cells` either way of the line nearest `around`, so the grid
/// is always under the view and never moves with it. Every tenth line is
/// brighter, as Unity's are.
pub fn grid_draws(
    arm: MeshHandle,
    around: Vec3,
    axis: usize,
    spacing: f32,
    cells: i32,
    thickness: f32,
) -> Vec<Draw> {
    let spacing = if spacing.is_finite() && spacing > 0.0 {
        spacing
    } else {
        1.0
    };
    let axis = axis.min(2);
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let unit = |i: usize| Vec3::AXES[i];
    let snap = |x: f32| (x / spacing).round() as i32;
    let (cu, cv) = (snap(around[u]), snap(around[v]));
    let reach = cells as f32 * spacing;
    let mut minor = Vec::new();
    let mut major = Vec::new();
    for (along, across, centre, other) in [(u, v, cu, cv), (v, u, cv, cu)] {
        let middle = other as f32 * spacing;
        for k in centre - cells..=centre + cells {
            let at = unit(along) * (k as f32 * spacing);
            let a = at + unit(across) * (middle - reach);
            let b = at + unit(across) * (middle + reach);
            if k % 10 == 0 {
                major.push((a, b));
            } else {
                minor.push((a, b));
            }
        }
    }
    let mut out = line_draws(
        arm,
        minor,
        Mat4::IDENTITY,
        thickness,
        Material::new(0.16, 0.16, 0.16).unlit(),
    );
    out.extend(line_draws(
        arm,
        major,
        Mat4::IDENTITY,
        thickness * 1.6,
        Material::new(0.3, 0.3, 0.3).unlit(),
    ));
    out
}

/// Line segments in `frame`'s space as thin boxes, `arm` a unit cube and
/// `thickness` a line's width in metres: how every outline here is drawn,
/// and how a module draws one of its own (the physics module's colliders).
pub fn line_draws(
    arm: MeshHandle,
    segments: Vec<(Vec3, Vec3)>,
    frame: Mat4,
    thickness: f32,
    material: Material,
) -> Vec<Draw> {
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
/// drag does cannot disagree. [`Gizmo`] and [`Drag`] do this themselves.
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
        Motion::Resize { scale, about } => Motion::Resize {
            scale,
            about: origin + orientation * (about - origin),
        },
    }
}

/// The move tool's draw list, on the world's axes, with `active` held.
pub fn draws(
    meshes: &GizmoMeshes,
    camera: &Camera,
    style: &GizmoStyle,
    origin: Vec3,
    active: Option<Handle>,
) -> Vec<Draw> {
    draws_for(Tool::Move, meshes, camera, style, origin, active)
}

/// A tool's draw list, on the world's axes, with `active` held.
pub fn draws_for(
    tool: Tool,
    meshes: &GizmoMeshes,
    camera: &Camera,
    style: &GizmoStyle,
    origin: Vec3,
    active: Option<Handle>,
) -> Vec<Draw> {
    let held = active.map(|handle| Grip::new(drag_of(tool, handle), handle));
    Gizmo::new(tool, camera, style, origin).draws(
        meshes,
        &Shown {
            held,
            ..Shown::default()
        },
    )
}

/// The drag a handle of a tool starts, where the handle alone says: the
/// Transform tool's X is its arrow.
fn drag_of(tool: Tool, handle: Handle) -> Tool {
    match (tool, handle) {
        (Tool::Transform, Handle::View | Handle::Free) => Tool::Rotate,
        (Tool::Transform, _) => Tool::Move,
        (tool, _) => tool,
    }
}

/// Which move handle a ray passes close enough to.
pub fn hit(
    camera: &Camera,
    style: &GizmoStyle,
    origin: Vec3,
    ray_origin: Vec3,
    ray_direction: Vec3,
) -> Option<Handle> {
    hit_for(Tool::Move, camera, style, origin, ray_origin, ray_direction)
}

/// Which handle of a tool a ray hits, on the world's axes.
pub fn hit_for(
    tool: Tool,
    camera: &Camera,
    style: &GizmoStyle,
    origin: Vec3,
    ray_origin: Vec3,
    ray_direction: Vec3,
) -> Option<Handle> {
    Gizmo::new(tool, camera, style, origin)
        .pick(ray_origin, ray_direction)
        .map(|grip| grip.handle)
}

/// Start a move drag on a handle.
pub fn begin(origin: Vec3, handle: Handle, ray_origin: Vec3, ray_direction: Vec3) -> Drag {
    begin_for(Tool::Move, origin, handle, ray_origin, ray_direction)
}

/// Start a drag with a given tool, on the world's axes, the view taken to
/// be along the ray.
pub fn begin_for(
    tool: Tool,
    origin: Vec3,
    handle: Handle,
    ray_origin: Vec3,
    ray_direction: Vec3,
) -> Drag {
    let camera = Camera {
        position: ray_origin,
        target: ray_origin + ray_direction,
        up: if ray_direction.normalize_or_zero().y.abs() > 0.99 {
            Vec3::Z
        } else {
            Vec3::Y
        },
        ..Camera::default()
    };
    Gizmo::new(tool, &camera, &GizmoStyle::default(), origin).begin(
        Grip::new(drag_of(tool, handle), handle),
        ray_origin,
        ray_direction,
    )
}

/// What a drag asks for now, given where the cursor's ray is.
pub fn update_for(drag: &Drag, ray_origin: Vec3, ray_direction: Vec3) -> Motion {
    drag.motion(ray_origin, ray_direction)
}

/// Where the gizmo should be now, given where the cursor's ray is.
///
/// Returns the new position rather than a delta: a caller that accumulates
/// deltas accumulates their rounding too, and a long drag drifts.
pub fn update(drag: &Drag, ray_origin: Vec3, ray_direction: Vec3) -> Vec3 {
    match drag.motion(ray_origin, ray_direction) {
        Motion::Position(p) => p,
        _ => drag.origin,
    }
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

    /// Looking straight down, the top of the picture toward −z.
    fn above() -> Camera {
        Camera {
            position: Vec3::new(0.0, 10.0, 0.0),
            target: Vec3::ZERO,
            up: Vec3::NEG_Z,
            ..Camera::default()
        }
    }

    /// The ray from a camera's eye through a point.
    fn aim(camera: &Camera, at: Vec3) -> (Vec3, Vec3) {
        (camera.position, (at - camera.position).normalize())
    }

    fn moved(motion: Motion) -> Vec3 {
        match motion {
            Motion::Position(p) => p,
            other => panic!("expected a move, got {other:?}"),
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
        let from = Vec3::new(length * 0.6, 0.0, 5.0);
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
        let gizmo = Gizmo::new(Tool::Move, &camera, &style, origin).turned(turn);
        let (from, direction) = aim(&camera, turn * Vec3::X * length * 0.7);
        let grip = gizmo.pick(from, direction);
        assert_eq!(grip, Some(Grip::new(Tool::Move, Handle::X)));
        let drag = gizmo.begin(grip.unwrap(), from, direction);
        let (from, direction) = aim(&camera, turn * Vec3::X * length * 1.7);
        let p = moved(drag.motion(from, direction));
        assert!(p.z < -0.1 && p.x.abs() < 1e-3 && p.y.abs() < 1e-3, "{p}");
        // And drawn where it is grabbed: the X arrowhead out along −z.
        let drawn = gizmo.draws(&GizmoMeshes::TEST, &Shown::default());
        let red = Handle::X.color().base_color;
        assert!(
            drawn.iter().any(|d| d.material.shading == Shading::Lit
                && d.material.base_color == red
                && d.transform.w_axis.z < -0.5 * length),
            "drawn where it is grabbed"
        );
    }

    #[test]
    fn a_camera_is_drawn_opening_the_way_it_looks() {
        let lens = crate::scene::Lens {
            fov_deg: 90.0,
            priority: 0,
            ortho: None,
            follow: None,
        };
        // Turned round: it looks along the world's −z.
        let placed = Mat4::from_rotation_translation(
            Quat::from_rotation_y(std::f32::consts::PI),
            Vec3::new(0.0, 1.0, 5.0),
        );
        let draws = camera_draws(
            MeshHandle::TEST,
            lens,
            placed,
            2.0,
            3.0,
            0.01,
            camera_color(),
        );
        assert_eq!(
            draws.len(),
            10,
            "four far edges, four from the eye, the up mark"
        );
        let farthest = draws
            .iter()
            .map(|d| d.transform.w_axis.z)
            .fold(f32::MAX, f32::min);
        assert!(farthest < 5.0 - 1.4, "out along −z: {farthest}");
        let ortho = crate::scene::Lens {
            ortho: Some(4.0),
            ..lens
        };
        let boxed = camera_draws(
            MeshHandle::TEST,
            ortho,
            placed,
            2.0,
            3.0,
            0.01,
            camera_color(),
        );
        assert_eq!(boxed.len(), 14, "a box: the near face too");
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
        let drag = begin(Vec3::ZERO, Handle::X, Vec3::new(-10.0, 0.0, 0.0), Vec3::X);
        let moved = update(&drag, Vec3::new(-10.0, 0.0, 0.0), Vec3::X);
        assert_eq!(moved, Vec3::ZERO);
    }

    #[test]
    fn a_plane_handle_keeps_the_grabbed_point_under_the_cursor() {
        // Seen from above and a little in front, the XZ square (across Y).
        let camera = Camera {
            position: Vec3::new(3.0, 8.0, 6.0),
            target: Vec3::ZERO,
            ..Camera::default()
        };
        let style = GizmoStyle::default();
        let origin = Vec3::new(1.0, 0.5, -2.0);
        let length = style.arm_length(&camera, origin);
        let gizmo = Gizmo::new(Tool::Move, &camera, &style, origin);
        // The square is in the quarter facing the eye: +x, +z.
        let grab = origin + Vec3::new(0.2, 0.0, 0.2) * length;
        let (o, d) = aim(&camera, grab);
        let grip = gizmo.pick(o, d);
        assert_eq!(grip, Some(Grip::new(Tool::Move, Handle::PlaneY)));
        let drag = gizmo.begin(grip.unwrap(), o, d);
        assert!((moved(drag.motion(o, d)) - origin).length() < 1e-4, "a grab is not a move");
        // Wherever the cursor goes on the plane, the grabbed point follows
        // it there, and nothing leaves the plane.
        let to = grab + Vec3::new(-1.3, 0.0, 0.7);
        let (o, d) = aim(&camera, to);
        let p = moved(drag.motion(o, d));
        assert!((p - (origin + (to - grab))).length() < 1e-3, "{p}");
        assert!((p.y - origin.y).abs() < 1e-4);
    }

    #[test]
    fn plane_squares_turn_to_the_quarter_facing_the_eye() {
        let style = GizmoStyle::default();
        let square = |camera: &Camera| {
            let gizmo = Gizmo::new(Tool::Move, camera, &style, Vec3::ZERO);
            let length = style.arm_length(camera, Vec3::ZERO);
            [1.0f32, -1.0].map(|side| {
                let (o, d) = aim(camera, Vec3::new(side, 0.0, side) * 0.2 * length);
                gizmo.pick(o, d)
            })
        };
        let front = Camera {
            position: Vec3::new(4.0, 6.0, 4.0),
            target: Vec3::ZERO,
            ..Camera::default()
        };
        let back = Camera {
            position: Vec3::new(-4.0, 6.0, -4.0),
            ..front
        };
        let y = Some(Grip::new(Tool::Move, Handle::PlaneY));
        assert_eq!(square(&front)[0], y, "toward +x +z from there");
        assert_ne!(square(&front)[1], y);
        assert_eq!(square(&back)[1], y, "and toward −x −z from behind");
        assert_ne!(square(&back)[0], y);
    }

    #[test]
    fn the_middle_slides_in_the_views_plane() {
        let style = GizmoStyle::default();
        let gizmo = Gizmo::new(Tool::Move, &camera(), &style, Vec3::ZERO);
        let (o, d) = aim(&camera(), Vec3::ZERO);
        let grip = gizmo.pick(o, d);
        assert_eq!(grip, Some(Grip::new(Tool::Move, Handle::Center)));
        let drag = gizmo.begin(grip.unwrap(), o, d);
        let (o, d) = aim(&camera(), Vec3::new(0.7, -0.4, 0.0));
        let p = moved(drag.motion(o, d));
        assert!((p - Vec3::new(0.7, -0.4, 0.0)).length() < 1e-4, "{p}");
    }

    #[test]
    fn hover_picks_by_priority_where_handles_overlap() {
        let style = GizmoStyle::default();
        let camera = Camera {
            position: Vec3::new(5.0, 4.0, 8.0),
            target: Vec3::ZERO,
            ..Camera::default()
        };
        let length = style.arm_length(&camera, Vec3::ZERO);
        let gizmo = Gizmo::new(Tool::Move, &camera, &style, Vec3::ZERO);
        // Just off the middle along X: inside the middle square and within
        // the X arm's reach — the middle wins.
        let (o, d) = aim(&camera, Vec3::X * 0.06 * length);
        assert_eq!(gizmo.pick(o, d).map(|g| g.handle), Some(Handle::Center));
        // On the square's edge by the X arm: the square wins over the arm.
        let (o, d) = aim(&camera, Vec3::new(0.2, 0.0, 0.11) * length);
        assert_eq!(gizmo.pick(o, d).map(|g| g.handle), Some(Handle::PlaneY));
        // Out along the arm, alone: the arm.
        let (o, d) = aim(&camera, Vec3::X * 0.6 * length);
        assert_eq!(gizmo.pick(o, d).map(|g| g.handle), Some(Handle::X));
        // Rotate: a ring over the ball.
        let rotate = Gizmo { tool: Tool::Rotate, ..gizmo };
        let (o, d) = aim(&camera, Vec3::new(0.0, 0.0, length));
        assert!(rotate.pick(o, d).is_some());
        assert_ne!(rotate.pick(o, d).map(|g| g.handle), Some(Handle::Free));
        let (o, d) = aim(&camera, Vec3::ZERO);
        assert_eq!(rotate.pick(o, d).map(|g| g.handle), Some(Handle::Free));
    }

    #[test]
    fn an_arm_pointing_at_the_eye_fades_and_cannot_be_grabbed() {
        let style = GizmoStyle::default();
        // Almost straight down the Z axis.
        let camera = Camera {
            position: Vec3::new(0.05, 0.05, 10.0),
            target: Vec3::ZERO,
            ..Camera::default()
        };
        let length = style.arm_length(&camera, Vec3::ZERO);
        for tool in [Tool::Move, Tool::Scale] {
            let gizmo = Gizmo::new(tool, &camera, &style, Vec3::ZERO);
            let (o, d) = aim(&camera, Vec3::Z * 0.5 * length);
            assert_ne!(gizmo.pick(o, d).map(|g| g.handle), Some(Handle::Z), "{tool:?}");
            let blue = Handle::Z.color().base_color;
            assert!(
                !gizmo
                    .draws(&GizmoMeshes::TEST, &Shown::default())
                    .iter()
                    .any(|d| d.material.base_color == blue && d.material.shading == Shading::Lit),
                "{tool:?}: an arm seen end-on is not drawn"
            );
        }
        // From the side it is there.
        let side = Gizmo::new(Tool::Move, &Camera { position: Vec3::new(8.0, 3.0, 5.0), ..camera }, &style, Vec3::ZERO);
        let blue = Handle::Z.color().base_color;
        assert!(side
            .draws(&GizmoMeshes::TEST, &Shown::default())
            .iter()
            .any(|d| d.material.base_color == blue && d.material.alpha >= 1.0));
    }

    /// Degrees of turn a rotate drag asks for, measured about `about`.
    fn turned_about(motion: Motion, about: Vec3) -> f32 {
        match motion {
            Motion::Rotation(q) => {
                let (axis, angle) = q.to_axis_angle();
                // `to_axis_angle` may hand back the opposite axis with the
                // opposite angle; measured against one axis so the sign
                // means the same thing in every case here.
                if angle.abs() < 1e-6 {
                    return 0.0;
                }
                angle.to_degrees() * axis.dot(about).signum()
            }
            other => panic!("expected a rotation, got {other:?}"),
        }
    }

    fn turned(motion: Motion) -> f32 {
        turned_about(motion, Vec3::Y)
    }

    #[test]
    fn a_ring_is_grabbed_where_it_is_drawn_and_the_middle_is_the_ball() {
        // Seen from above the Y ring is a whole circle.
        let style = GizmoStyle::default();
        let radius = style.arm_length(&above(), Vec3::ZERO);
        let gizmo = Gizmo::new(Tool::Rotate, &above(), &style, Vec3::ZERO);
        let (o, d) = aim(&above(), Vec3::new(radius, 0.0, 0.0));
        assert_eq!(gizmo.pick(o, d), Some(Grip::new(Tool::Rotate, Handle::Y)));
        // Well inside the rim: the ball, for a free turn.
        let (o, d) = aim(&above(), Vec3::new(0.3 * radius, 0.0, 0.2 * radius));
        assert_eq!(gizmo.pick(o, d), Some(Grip::new(Tool::Rotate, Handle::Free)));
        // The outer ring, about the line of sight.
        let (o, d) = aim(&above(), Vec3::new(0.0, 0.0, 1.1 * radius));
        assert_eq!(gizmo.pick(o, d), Some(Grip::new(Tool::Rotate, Handle::View)));
    }

    #[test]
    fn only_the_near_half_of_a_ring_is_drawn_or_grabbed() {
        let style = GizmoStyle::default();
        // From in front and above: the Y ring's far half is behind the ball.
        let camera = Camera {
            position: Vec3::new(0.0, 5.0, 10.0),
            target: Vec3::ZERO,
            ..Camera::default()
        };
        let radius = style.arm_length(&camera, Vec3::ZERO);
        let gizmo = Gizmo::new(Tool::Rotate, &camera, &style, Vec3::ZERO);
        let green = Handle::Y.color().base_color;
        let ring: Vec<Vec3> = gizmo
            .draws(&GizmoMeshes::TEST, &Shown::default())
            .into_iter()
            .filter(|d| d.material.base_color == green)
            .map(|d| d.transform.w_axis.truncate())
            .collect();
        assert!(!ring.is_empty());
        let to_eye = (camera.position).normalize();
        assert!(
            ring.iter().all(|p| p.dot(to_eye) > -0.05 * radius),
            "no piece of the far half"
        );
        let (o, d) = aim(&camera, Vec3::new(0.0, 0.0, radius));
        assert_eq!(gizmo.pick(o, d).map(|g| g.handle), Some(Handle::Y), "the near side");
        let (o, d) = aim(&camera, Vec3::new(0.0, 0.0, -radius));
        assert_ne!(gizmo.pick(o, d).map(|g| g.handle), Some(Handle::Y), "the far side");
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
        // In steps, it lands on a multiple of the step.
        let near = Vec3::new(radius * 0.8f32.cos(), 4.0, -radius * 0.8f32.sin());
        let stepped = turned(drag.motion_snapped(near, Vec3::NEG_Y, 15.0));
        assert!((stepped / 15.0 - (stepped / 15.0).round()).abs() < 1e-3, "{stepped}");
        assert!((stepped.abs() - 45.0).abs() < 1e-3, "0.8 rad is nearest 45°: {stepped}");
    }

    #[test]
    fn a_ring_seen_edge_on_turns_along_its_tangent() {
        // From in front, the Y ring is a line: a drag across the view
        // along it still turns, by the arc dragged.
        let style = GizmoStyle::default();
        let camera = Camera {
            position: Vec3::new(0.0, 0.3, 10.0),
            target: Vec3::ZERO,
            ..Camera::default()
        };
        let radius = style.arm_length(&camera, Vec3::ZERO);
        let gizmo = Gizmo::new(Tool::Rotate, &camera, &style, Vec3::ZERO);
        let a = 30f32.to_radians();
        let grab = Vec3::new(a.sin(), 0.0, a.cos()) * radius;
        let (o, d) = aim(&camera, grab);
        let grip = gizmo.pick(o, d).unwrap();
        assert_eq!(grip.handle, Handle::Y);
        let drag = gizmo.begin(grip, o, d);
        let tangent = Vec3::new(a.cos(), 0.0, -a.sin());
        let (o, d) = aim(&camera, grab + tangent * radius * 0.5);
        let angle = turned(drag.motion(o, d));
        assert!((angle.abs() - 28.6).abs() < 3.0, "half a radius of arc: {angle}");
    }

    #[test]
    fn the_view_ring_turns_about_the_line_of_sight() {
        let style = GizmoStyle::default();
        let radius = style.arm_length(&camera(), Vec3::ZERO) * 1.1;
        let gizmo = Gizmo::new(Tool::Rotate, &camera(), &style, Vec3::ZERO);
        let (o, d) = aim(&camera(), Vec3::new(radius, 0.0, 0.0));
        let grip = gizmo.pick(o, d);
        assert_eq!(grip, Some(Grip::new(Tool::Rotate, Handle::View)));
        let drag = gizmo.begin(grip.unwrap(), o, d);
        let (o, d) = aim(&camera(), Vec3::new(0.0, radius, 0.0));
        let motion = drag.motion(o, d);
        let Motion::Rotation(q) = motion else {
            panic!("{motion:?}")
        };
        // A quarter turn about the line of sight takes +x to +y.
        assert!((q * Vec3::X - Vec3::Y).length() < 1e-3, "{}", q * Vec3::X);
        assert!((turned_about(motion, Vec3::Z).abs() - 90.0).abs() < 0.5);
        let swept = drag.swept(drag.angle(o, d).unwrap()).unwrap();
        assert!((swept.radius - radius).abs() < 0.05 * radius);
    }

    #[test]
    fn the_ball_turns_the_grabbed_point_to_the_cursor() {
        let style = GizmoStyle::default();
        let radius = style.arm_length(&camera(), Vec3::ZERO);
        let gizmo = Gizmo::new(Tool::Rotate, &camera(), &style, Vec3::ZERO);
        let (o, d) = aim(&camera(), Vec3::new(0.2 * radius, 0.1 * radius, 0.0));
        let grip = gizmo.pick(o, d);
        assert_eq!(grip, Some(Grip::new(Tool::Rotate, Handle::Free)));
        let drag = gizmo.begin(grip.unwrap(), o, d);
        let on_ball = |x: f32, y: f32| {
            Vec3::new(x, y, (radius * radius - x * x - y * y).sqrt())
        };
        let from = on_ball(0.2 * radius, 0.1 * radius);
        let to = on_ball(-0.3 * radius, 0.4 * radius);
        let (o, d) = aim(&camera(), Vec3::new(to.x, to.y, 0.0));
        let Motion::Rotation(q) = drag.motion(o, d) else {
            panic!("a turn")
        };
        // Near enough: the eye is ten metres off, not infinitely far.
        let turned = q * from.normalize();
        assert!((turned - to.normalize()).length() < 0.05, "{turned} vs {}", to.normalize());
    }

    #[test]
    fn the_middle_cube_scales_every_axis_alike() {
        let style = GizmoStyle::default();
        let length = style.arm_length(&camera(), Vec3::ZERO);
        let gizmo = Gizmo::new(Tool::Scale, &camera(), &style, Vec3::ZERO);
        let (o, d) = aim(&camera(), Vec3::ZERO);
        let grip = gizmo.pick(o, d);
        assert_eq!(grip, Some(Grip::new(Tool::Scale, Handle::Center)));
        let drag = gizmo.begin(grip.unwrap(), o, d);
        assert_eq!(drag.motion(o, d), Motion::Scale(Vec3::ONE));
        let (o, d) = aim(&camera(), Vec3::X * length * 0.5);
        match drag.motion(o, d) {
            Motion::Scale(f) => {
                assert!(f.x > 1.2 && f.x == f.y && f.y == f.z, "{f}");
            }
            other => panic!("{other:?}"),
        }
        let (o, d) = aim(&camera(), Vec3::NEG_X * length * 0.5);
        match drag.motion(o, d) {
            Motion::Scale(f) => assert!(f.x < 0.8 && f.x > 0.0, "{f}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_scale_arm_stretches_while_it_is_dragged() {
        let style = GizmoStyle::default();
        let gizmo = Gizmo::new(Tool::Scale, &camera(), &style, Vec3::ZERO);
        let reach = |stretch: Vec3| {
            gizmo
                .draws(
                    &GizmoMeshes::TEST,
                    &Shown {
                        stretch,
                        ..Shown::default()
                    },
                )
                .iter()
                .filter(|d| d.material.shading == Shading::Lit)
                .map(|d| d.transform.w_axis.x)
                .fold(f32::MIN, f32::max)
        };
        let normal = reach(Vec3::ONE);
        assert!((reach(Vec3::new(2.0, 1.0, 1.0)) - 2.0 * normal).abs() < 1e-4);
    }

    #[test]
    fn a_rect_corner_resizes_with_the_far_corner_staying_put() {
        let style = GizmoStyle::default();
        let origin = Vec3::new(0.0, 0.0, 0.0);
        // A box two by one, seen from in front: the rect is in XY.
        let (min, max) = (Vec3::new(-1.0, -0.5, -0.2), Vec3::new(1.0, 0.5, 0.2));
        let gizmo = Gizmo::new(Tool::Rect, &camera(), &style, origin).around(min, max);
        let corner = Vec3::new(1.0, 0.5, 0.0);
        let (o, d) = aim(&camera(), corner);
        let grip = gizmo.pick(o, d);
        assert_eq!(grip, Some(Grip::new(Tool::Rect, Handle::Rect(1, 1))));
        let drag = gizmo.begin(grip.unwrap(), o, d);
        let (o, d) = aim(&camera(), Vec3::new(2.0, 1.5, 0.0));
        let Motion::Resize { scale, about } = drag.motion(o, d) else {
            panic!("a resize")
        };
        let resize = |p: Vec3| about + scale * (p - about);
        let far = Vec3::new(-1.0, -0.5, 0.0);
        assert!((resize(far) - far).length() < 1e-3, "the far corner stays: {}", resize(far));
        assert!((resize(corner) - Vec3::new(2.0, 1.5, 0.0)).length() < 0.02, "{}", resize(corner));
        assert_eq!(scale.z, 1.0, "not across the rect");
        // An edge moves one side only.
        let (o, d) = aim(&camera(), Vec3::new(1.0, 0.1, 0.0));
        let grip = gizmo.pick(o, d);
        assert_eq!(grip, Some(Grip::new(Tool::Rect, Handle::Rect(1, 0))));
        let drag = gizmo.begin(grip.unwrap(), o, d);
        let (o, d) = aim(&camera(), Vec3::new(2.0, 0.6, 0.0));
        let Motion::Resize { scale, .. } = drag.motion(o, d) else {
            panic!("a resize")
        };
        assert!((scale.x - 1.5).abs() < 0.02 && scale.y == 1.0, "{scale}");
        // The inside moves it.
        let (o, d) = aim(&camera(), Vec3::new(-0.5, 0.2, 0.0));
        assert_eq!(gizmo.pick(o, d), Some(Grip::new(Tool::Rect, Handle::Rect(0, 0))));
    }

    #[test]
    fn the_transform_tool_hands_each_part_its_own_drag() {
        let style = GizmoStyle::default();
        let camera = Camera {
            position: Vec3::new(6.0, 5.0, 8.0),
            target: Vec3::ZERO,
            ..Camera::default()
        };
        let length = style.arm_length(&camera, Vec3::ZERO);
        let gizmo = Gizmo::new(Tool::Transform, &camera, &style, Vec3::ZERO);
        let at = |p: Vec3| {
            let (o, d) = aim(&camera, p);
            gizmo.pick(o, d)
        };
        assert_eq!(at(Vec3::X * 0.6 * length), Some(Grip::new(Tool::Move, Handle::X)));
        assert_eq!(at(Vec3::X * 0.98 * length), Some(Grip::new(Tool::Scale, Handle::X)));
        assert_eq!(at(Vec3::X * 1.25 * length), Some(Grip::new(Tool::Move, Handle::X)));
        let ring = Vec3::new(0.0, 0.8 * length * 0.6, 0.8 * length * 0.8);
        assert_eq!(at(ring), Some(Grip::new(Tool::Rotate, Handle::X)));
        assert_eq!(at(Vec3::ZERO), Some(Grip::new(Tool::Move, Handle::Center)));
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
        let camera = Camera {
            position: Vec3::new(5.0, 4.0, 8.0),
            target: Vec3::ZERO,
            ..Camera::default()
        };
        let at = |tool| draws_for(tool, &GizmoMeshes::TEST, &camera, &style, Vec3::ZERO, None);
        let cones = |tool| at(tool).iter().filter(|d| d.material.shading == Shading::Lit).count();
        assert_eq!(cones(Tool::Move), 3, "three arrowheads");
        assert_eq!(cones(Tool::Scale), 4, "three cubes and the middle one");
        assert_eq!(cones(Tool::Rotate), 0, "rings and a ball, all flat");
        assert_eq!(cones(Tool::Transform), 6, "arrowheads and cubes");
        let translucent = |tool| at(tool).iter().filter(|d| d.material.alpha < 1.0).count();
        assert!(translucent(Tool::Move) > 0, "the squares");
        assert!(translucent(Tool::Rotate) > 0, "the ball");
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
    fn the_hovered_and_held_handles_go_yellow() {
        let style = GizmoStyle::default();
        let camera = Camera {
            position: Vec3::new(5.0, 4.0, 8.0),
            target: Vec3::ZERO,
            ..Camera::default()
        };
        let gizmo = Gizmo::new(Tool::Move, &camera, &style, Vec3::ZERO);
        let hot = hot_color().base_color;
        let yellow = |shown: Shown| {
            gizmo
                .draws(&GizmoMeshes::TEST, &shown)
                .iter()
                .filter(|d| d.material.base_color == hot)
                .count()
        };
        assert_eq!(yellow(Shown::default()), 0);
        let y = Some(Grip::new(Tool::Move, Handle::Y));
        let x = Some(Grip::new(Tool::Move, Handle::X));
        let hovered = yellow(Shown {
            hovered: y,
            ..Shown::default()
        });
        assert_eq!(hovered, 2, "the Y shaft and arrowhead");
        // Held wins over hovered: one yellow handle at a time.
        let held = gizmo.draws(
            &GizmoMeshes::TEST,
            &Shown {
                hovered: y,
                held: x,
                ..Shown::default()
            },
        );
        let red = Handle::Y.color().base_color;
        assert!(held.iter().any(|d| d.material.base_color == red), "Y is its own colour");
    }

    #[test]
    fn a_turn_draws_the_pie_it_has_swept() {
        let style = GizmoStyle::default();
        let radius = style.arm_length(&above(), Vec3::ZERO);
        let gizmo = Gizmo::new(Tool::Rotate, &above(), &style, Vec3::ZERO);
        let (o, d) = aim(&above(), Vec3::new(radius, 0.0, 0.0));
        let grip = gizmo.pick(o, d).unwrap();
        let drag = gizmo.begin(grip, o, d);
        let (o, d) = aim(&above(), Vec3::new(0.0, 0.0, radius));
        let angle = drag.angle(o, d).unwrap();
        assert!((angle.to_degrees().abs() - 90.0).abs() < 1.0, "{}", angle.to_degrees());
        let plain = gizmo.draws(&GizmoMeshes::TEST, &Shown::default()).len();
        let swept = gizmo
            .draws(
                &GizmoMeshes::TEST,
                &Shown {
                    held: Some(grip),
                    swept: drag.swept(angle),
                    ..Shown::default()
                },
            )
            .len();
        assert!(swept > plain + 8, "a quarter of a disc's triangles: {plain} → {swept}");
    }

    #[test]
    fn the_grid_is_on_its_plane_around_the_view_and_does_not_slide() {
        let arm = MeshHandle::TEST;
        let draws = grid_draws(arm, Vec3::new(3.3, 7.0, -12.6), 1, 0.5, 4, 0.01);
        assert_eq!(draws.len(), 2 * 9, "nine lines each way");
        for d in &draws {
            let (_, _, at) = d.transform.to_scale_rotation_translation();
            assert!(at.y.abs() < 1e-5, "on the ground, not at the eye: {at}");
            // Lines stay on multiples of the spacing: the grid is the
            // world's, the view only chooses which part of it to show.
            let on = |x: f32| ((x / 0.5).round() * 0.5 - x).abs() < 1e-4;
            assert!(on(at.x) || on(at.z), "{at}");
        }
        let centre = draws
            .iter()
            .map(|d| d.transform.w_axis.truncate())
            .fold(Vec3::ZERO, |a, b| a + b)
            / draws.len() as f32;
        assert!(
            (centre - Vec3::new(3.5, 0.0, -12.5)).length() < 1e-3,
            "{centre}"
        );
        // A front view's grid stands on the wall it looks at.
        let wall = grid_draws(arm, Vec3::ZERO, 2, 1.0, 2, 0.01);
        assert!(wall.iter().all(|d| d.transform.w_axis.z.abs() < 1e-5));
    }
}
