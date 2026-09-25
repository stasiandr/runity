//! Two of parry's shapes as the engine needs them to behave: a convex hull
//! that stands on the whole of a broad face ([`Hull`]), and any shape on a
//! body that is never swept ([`Unswept`]).

use rapier3d::parry::bounding_volume::{Aabb, BoundingSphere};
use rapier3d::parry::mass_properties::MassProperties;
use rapier3d::parry::math::{Pose, Real, Vector};
use rapier3d::parry::query::{PointProjection, PointQuery, Ray, RayCast, RayIntersection};
use rapier3d::parry::shape::{
    CompositeShape, ConvexPolyhedron, FeatureId, PackedFeatureId, PolygonalFeature,
    PolygonalFeatureMap, Shape, ShapeType, SharedShape, SupportMap, TypedShape,
};

/// A convex hull that stands on the whole of a broad face.
///
/// parry clips contacts between two faces of at most four corners each. A
/// hull's face with more — the round bottom of a mine, a barrel's lid — it
/// cuts down to its *first four* corners, a sliver along one side of the
/// face (parry's own TODO: "select four vertices that maximize the area").
/// A disc set down on the ground then touches it along that sliver only:
/// it tips over the side the sliver is not on, and where the ground is a
/// mesh whose internal edges are fixed, it tips into the mesh and through.
///
/// PhysX clips the whole face and keeps the four contacts that span the
/// most area. This is parry's [`ConvexPolyhedron`] with that one thing
/// changed: a face of more than four corners is represented by the four of
/// its corners that span the most of it — the corner farthest from the
/// face's middle, the one farthest from that, and the farthest on each
/// side of the line between them. Which four depends only on the face, so
/// a body resting on it touches at the same points step after step.
#[derive(Clone, Debug)]
pub struct Hull {
    pub polyhedron: ConvexPolyhedron,
    /// Per face, the positions (in the face's own corner list) of the four
    /// corners that stand for it, in the face's order; `None` for a face
    /// of four corners or fewer, which is itself.
    quads: Vec<Option<[u32; 4]>>,
}

impl Hull {
    pub fn new(polyhedron: ConvexPolyhedron) -> Self {
        let quads = polyhedron
            .faces()
            .iter()
            .map(|face| {
                let first = face.first_vertex_or_edge as usize;
                let n = face.num_vertices_or_edges as usize;
                (n > 4).then(|| {
                    let corners: Vec<Vector> = polyhedron.vertices_adj_to_face()[first..first + n]
                        .iter()
                        .map(|v| polyhedron.points()[*v as usize])
                        .collect();
                    widest_quad(&corners, face.normal)
                })
            })
            .collect();
        Self { polyhedron, quads }
    }
}

/// The positions of four of `corners` (a convex polygon, in order, facing
/// `normal`) that span the most of it, in the polygon's order: the corner
/// farthest from its middle, the one farthest from that, and the farthest
/// from the line between them on each side — PhysX's pick of four contacts
/// out of a patch. Ties go to the earlier corner.
fn widest_quad(corners: &[Vector], normal: Vector) -> [u32; 4] {
    let middle = corners.iter().copied().sum::<Vector>() / corners.len() as Real;
    let farthest = |from: &dyn Fn(Vector) -> Real| {
        let mut best = 0;
        for (i, c) in corners.iter().enumerate() {
            if from(*c) > from(corners[best]) {
                best = i;
            }
        }
        best
    };
    let a = farthest(&|c| (c - middle).length_squared());
    let b = farthest(&|c| (c - corners[a]).length_squared());
    let across = normal.cross(corners[b] - corners[a]);
    let c = farthest(&|c| (c - corners[a]).dot(across));
    let d = farthest(&|c| -(c - corners[a]).dot(across));
    let mut quad = [a as u32, b as u32, c as u32, d as u32];
    quad.sort_unstable();
    quad
}

impl PolygonalFeatureMap for Hull {
    fn local_support_feature(&self, dir: Vector, out: &mut PolygonalFeature) {
        let hull = &self.polyhedron;
        let mut best = 0;
        let mut best_dot = hull.faces()[0].normal.dot(dir);
        for (i, face) in hull.faces().iter().enumerate().skip(1) {
            let dot = face.normal.dot(dir);
            if dot > best_dot {
                best = i;
                best_dot = dot;
            }
        }
        let face = &hull.faces()[best];
        let first = face.first_vertex_or_edge as usize;
        let n = face.num_vertices_or_edges as usize;
        let picked: Vec<usize> = match self.quads[best] {
            Some(quad) => quad.iter().map(|k| first + *k as usize).collect(),
            None => (first..first + n.min(4)).collect(),
        };
        for (i, at) in picked.iter().enumerate() {
            let vertex = hull.vertices_adj_to_face()[*at];
            out.vertices[i] = hull.points()[vertex as usize];
            out.vids[i] = PackedFeatureId::vertex(vertex);
            // The side leaving this corner — for a skipped stretch of the
            // face, the first side of it: a name to track the contact by.
            out.eids[i] = PackedFeatureId::edge(hull.edges_adj_to_face()[*at]);
        }
        out.fid = PackedFeatureId::face(best as u32);
        out.num_vertices = picked.len();
    }

    fn is_convex_polyhedron(&self) -> bool {
        true
    }
}

impl SupportMap for Hull {
    fn local_support_point(&self, dir: Vector) -> Vector {
        self.polyhedron.local_support_point(dir)
    }
}

impl RayCast for Hull {
    fn cast_local_ray_and_get_normal(
        &self,
        ray: &Ray,
        max_time_of_impact: Real,
        solid: bool,
    ) -> Option<RayIntersection> {
        self.polyhedron.cast_local_ray_and_get_normal(ray, max_time_of_impact, solid)
    }
}

impl PointQuery for Hull {
    fn project_local_point(&self, pt: Vector, solid: bool) -> PointProjection {
        self.polyhedron.project_local_point(pt, solid)
    }

    fn project_local_point_and_get_feature(&self, pt: Vector) -> (PointProjection, FeatureId) {
        self.polyhedron.project_local_point_and_get_feature(pt)
    }
}

impl Shape for Hull {
    fn compute_local_aabb(&self) -> Aabb {
        self.polyhedron.compute_local_aabb()
    }

    fn compute_local_bounding_sphere(&self) -> BoundingSphere {
        self.polyhedron.compute_local_bounding_sphere()
    }

    fn compute_aabb(&self, position: &Pose) -> Aabb {
        self.polyhedron.compute_aabb(position)
    }

    fn clone_dyn(&self) -> Box<dyn Shape> {
        Box::new(self.clone())
    }

    fn scale_dyn(&self, scale: Vector, _num_subdivisions: u32) -> Option<Box<dyn Shape>> {
        Some(Box::new(Hull::new(self.polyhedron.clone().scaled(scale)?)))
    }

    fn mass_properties(&self, density: Real) -> MassProperties {
        self.polyhedron.mass_properties(density)
    }

    fn is_convex(&self) -> bool {
        true
    }

    fn shape_type(&self) -> ShapeType {
        ShapeType::Custom
    }

    fn as_typed_shape(&self) -> TypedShape<'_> {
        TypedShape::Custom(self)
    }

    fn ccd_thickness(&self) -> Real {
        self.polyhedron.ccd_thickness()
    }

    fn ccd_angular_thickness(&self) -> Real {
        self.polyhedron.ccd_angular_thickness()
    }

    fn as_support_map(&self) -> Option<&dyn SupportMap> {
        Some(self as &dyn SupportMap)
    }

    fn as_polygonal_feature_map(&self) -> Option<(&dyn PolygonalFeatureMap, Real)> {
        Some((self as &dyn PolygonalFeatureMap, 0.0))
    }

    fn feature_normal_at_point(
        &self,
        subshape: u32,
        feature: FeatureId,
        point: Vector,
    ) -> Option<Vector> {
        self.polyhedron.feature_normal_at_point(subshape, feature, point)
    }
}

/// Any shape, on a body that is never swept between steps.
///
/// rapier sweeps every dynamic body that moves more than half its
/// thickness in a step against what stands still, whether or not it asked
/// to be swept, and stops it at the first thing the sweep meets. A body
/// that moves itself over the ground it rests on — a sprinting pawn — meets
/// the next rise of the ground ahead each step and is stopped short of
/// where it was going, its speed untouched; PhysX's sweep lets it slide on.
/// A shape reports how thin it is, and rapier sweeps a body only when it
/// outruns its thinnest shape: this one is the shape it wraps in every way
/// but that, and says it is not thin at all.
#[derive(Clone)]
pub struct Unswept(pub SharedShape);

impl RayCast for Unswept {
    fn cast_local_ray_and_get_normal(
        &self,
        ray: &Ray,
        max_time_of_impact: Real,
        solid: bool,
    ) -> Option<RayIntersection> {
        self.0.cast_local_ray_and_get_normal(ray, max_time_of_impact, solid)
    }
}

impl PointQuery for Unswept {
    fn project_local_point(&self, pt: Vector, solid: bool) -> PointProjection {
        self.0.project_local_point(pt, solid)
    }

    fn project_local_point_and_get_feature(&self, pt: Vector) -> (PointProjection, FeatureId) {
        self.0.project_local_point_and_get_feature(pt)
    }
}

impl Shape for Unswept {
    fn compute_local_aabb(&self) -> Aabb {
        self.0.compute_local_aabb()
    }

    fn compute_local_bounding_sphere(&self) -> BoundingSphere {
        self.0.compute_local_bounding_sphere()
    }

    fn compute_aabb(&self, position: &Pose) -> Aabb {
        self.0.compute_aabb(position)
    }

    fn clone_dyn(&self) -> Box<dyn Shape> {
        Box::new(self.clone())
    }

    fn scale_dyn(&self, scale: Vector, num_subdivisions: u32) -> Option<Box<dyn Shape>> {
        let scaled = self.0.scale_dyn(scale, num_subdivisions)?;
        Some(Box::new(Unswept(SharedShape(scaled.into()))))
    }

    fn mass_properties(&self, density: Real) -> MassProperties {
        self.0.mass_properties(density)
    }

    fn is_convex(&self) -> bool {
        self.0.is_convex()
    }

    fn shape_type(&self) -> ShapeType {
        ShapeType::Custom
    }

    fn as_typed_shape(&self) -> TypedShape<'_> {
        TypedShape::Custom(self)
    }

    /// Not thin at all: never outrun in a step, so never swept.
    fn ccd_thickness(&self) -> Real {
        Real::MAX
    }

    fn ccd_angular_thickness(&self) -> Real {
        self.0.ccd_angular_thickness()
    }

    fn as_support_map(&self) -> Option<&dyn SupportMap> {
        self.0.as_support_map()
    }

    fn as_composite_shape(&self) -> Option<&(dyn CompositeShape + Sync)> {
        self.0.as_composite_shape()
    }

    fn as_polygonal_feature_map(&self) -> Option<(&dyn PolygonalFeatureMap, Real)> {
        self.0.as_polygonal_feature_map()
    }

    fn feature_normal_at_point(
        &self,
        subshape: u32,
        feature: FeatureId,
        point: Vector,
    ) -> Option<Vector> {
        self.0.feature_normal_at_point(subshape, feature, point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A disc's round face stands for itself by a square across it, not a
    /// sliver along its rim.
    #[test]
    fn a_round_face_is_stood_on_across_its_width() {
        let rim: Vec<Vector> = (0..24)
            .map(|i| {
                let a = i as Real / 24.0 * std::f32::consts::TAU;
                Vector::new(a.cos(), 0.0, a.sin())
            })
            .collect();
        let quad = widest_quad(&rim, Vector::NEG_Y);
        let [a, b, c, d] = quad.map(|k| rim[k as usize]);
        // A square in the unit circle has the area 2; parry's first four of
        // 24 corners, about 0.13.
        let area = 0.5 * ((c - a).cross(d - b)).length();
        assert!(area > 1.9, "{quad:?} spans {area}");
        assert!(quad.windows(2).all(|w| w[0] < w[1]), "in the face's order");
    }
}
