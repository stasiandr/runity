//! Collision shapes, their bounds, and how much they weigh.
//!
//! Four primitives cover most of what a game needs: a sphere, a box, a capsule
//! (a segment with a radius — the shape almost every character controller
//! wants), and a half-space for the ground. Each one has to answer three
//! questions: where is it, how heavy is it, and how hard is it to spin.

use runity_math::{Mat3, Quat, Vec3};

/// A rigid transform: rotation then translation, no scale.
///
/// Physics has no use for scale — a scaled collider is a different collider —
/// and leaving it out keeps inertia tensors meaningful.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Isometry {
    pub position: Vec3,
    pub rotation: Quat,
}

impl Default for Isometry {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Isometry {
    pub const IDENTITY: Isometry = Isometry {
        position: Vec3::ZERO,
        rotation: Quat::IDENTITY,
    };

    pub fn new(position: Vec3, rotation: Quat) -> Self {
        Self { position, rotation }
    }

    pub fn from_position(position: Vec3) -> Self {
        Self {
            position,
            rotation: Quat::IDENTITY,
        }
    }

    #[inline]
    pub fn transform_point(&self, local: Vec3) -> Vec3 {
        self.rotation.rotate(local) + self.position
    }

    #[inline]
    pub fn transform_direction(&self, local: Vec3) -> Vec3 {
        self.rotation.rotate(local)
    }

    #[inline]
    pub fn inverse_transform_point(&self, world: Vec3) -> Vec3 {
        self.rotation.conjugate().rotate(world - self.position)
    }

    #[inline]
    pub fn inverse_transform_direction(&self, world: Vec3) -> Vec3 {
        self.rotation.conjugate().rotate(world)
    }

    pub fn inverse(&self) -> Isometry {
        let rotation = self.rotation.conjugate();
        Isometry {
            position: rotation.rotate(-self.position),
            rotation,
        }
    }
}

/// Axis-aligned bounding box, in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    pub fn from_center_half_extents(center: Vec3, half: Vec3) -> Self {
        Self {
            min: center - half,
            max: center + half,
        }
    }

    /// Covers everything — what an infinite plane reports.
    pub fn infinite() -> Self {
        Self {
            min: Vec3::splat(f32::NEG_INFINITY),
            max: Vec3::splat(f32::INFINITY),
        }
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    /// Grow by `margin` on every side — used to keep contacts alive for a frame
    /// or two after they stop touching, which is what warm starting needs.
    pub fn expanded(&self, margin: f32) -> Self {
        Self {
            min: self.min - Vec3::splat(margin),
            max: self.max + Vec3::splat(margin),
        }
    }

    pub fn overlaps(&self, other: &Aabb) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
            && self.min.z <= other.max.z
            && self.max.z >= other.min.z
    }

    pub fn contains(&self, point: Vec3) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
            && point.z >= self.min.z
            && point.z <= self.max.z
    }

    pub fn merged(&self, other: &Aabb) -> Aabb {
        Aabb {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }
}

/// Mass, center of mass, and the inertia tensor in the shape's own axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MassProperties {
    pub mass: f32,
    /// Diagonal of the inertia tensor in local axes. All four primitives here
    /// are symmetric enough that their principal axes are the local ones.
    pub inertia: Vec3,
}

impl MassProperties {
    pub const ZERO: MassProperties = MassProperties {
        mass: 0.0,
        inertia: Vec3::ZERO,
    };

    pub fn local_tensor(&self) -> Mat3 {
        Mat3::from_diagonal(self.inertia)
    }

    /// Inverse inertia in local axes; zero on any axis that cannot rotate.
    pub fn inverse_local_tensor(&self) -> Mat3 {
        let inv = |v: f32| if v > 0.0 { 1.0 / v } else { 0.0 };
        Mat3::from_diagonal(Vec3::new(
            inv(self.inertia.x),
            inv(self.inertia.y),
            inv(self.inertia.z),
        ))
    }
}

/// A collision primitive, in its own local frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Sphere {
        radius: f32,
    },
    /// Box centered on the origin.
    Cuboid {
        half_extents: Vec3,
    },
    /// Segment along the local Y axis, rounded by `radius`.
    Capsule {
        /// Half the length of the straight part, excluding the caps.
        half_height: f32,
        radius: f32,
    },
    /// Everything below the plane is solid. Static only — an infinite object
    /// has no meaningful mass.
    HalfSpace {
        normal: Vec3,
    },
}

impl Shape {
    pub fn sphere(radius: f32) -> Self {
        Shape::Sphere { radius }
    }

    pub fn cuboid(half_extents: Vec3) -> Self {
        Shape::Cuboid { half_extents }
    }

    pub fn capsule(half_height: f32, radius: f32) -> Self {
        Shape::Capsule {
            half_height,
            radius,
        }
    }

    /// The ground: a half-space with its surface through the origin.
    pub fn ground() -> Self {
        Shape::HalfSpace { normal: Vec3::Y }
    }

    /// World-space bounds of this shape placed at `at`.
    pub fn aabb(&self, at: &Isometry) -> Aabb {
        match *self {
            Shape::Sphere { radius } => {
                Aabb::from_center_half_extents(at.position, Vec3::splat(radius))
            }
            Shape::Cuboid { half_extents } => {
                // Project the box's axes onto the world axes: the extent along
                // world X is how far the three rotated axes reach along it.
                let m = Mat3::from_quat(at.rotation);
                let half = Vec3::new(
                    m.cols[0].x.abs() * half_extents.x
                        + m.cols[1].x.abs() * half_extents.y
                        + m.cols[2].x.abs() * half_extents.z,
                    m.cols[0].y.abs() * half_extents.x
                        + m.cols[1].y.abs() * half_extents.y
                        + m.cols[2].y.abs() * half_extents.z,
                    m.cols[0].z.abs() * half_extents.x
                        + m.cols[1].z.abs() * half_extents.y
                        + m.cols[2].z.abs() * half_extents.z,
                );
                Aabb::from_center_half_extents(at.position, half)
            }
            Shape::Capsule {
                half_height,
                radius,
            } => {
                let axis = at.transform_direction(Vec3::Y) * half_height;
                let (a, b) = (at.position - axis, at.position + axis);
                Aabb::new(
                    a.min(b) - Vec3::splat(radius),
                    a.max(b) + Vec3::splat(radius),
                )
            }
            Shape::HalfSpace { .. } => Aabb::infinite(),
        }
    }

    /// Mass and inertia for a uniform body of this shape.
    pub fn mass_properties(&self, density: f32) -> MassProperties {
        const PI: f32 = core::f32::consts::PI;
        match *self {
            Shape::Sphere { radius } => {
                let mass = 4.0 / 3.0 * PI * radius.powi(3) * density;
                let inertia = 0.4 * mass * radius * radius;
                MassProperties {
                    mass,
                    inertia: Vec3::splat(inertia),
                }
            }
            Shape::Cuboid { half_extents } => {
                let h = half_extents;
                let mass = 8.0 * h.x * h.y * h.z * density;
                // m/3 * (hy² + hz²) — the m/12 (w² + d²) form with half extents.
                let factor = mass / 3.0;
                MassProperties {
                    mass,
                    inertia: Vec3::new(
                        factor * (h.y * h.y + h.z * h.z),
                        factor * (h.x * h.x + h.z * h.z),
                        factor * (h.x * h.x + h.y * h.y),
                    ),
                }
            }
            Shape::Capsule {
                half_height,
                radius,
            } => {
                // A cylinder plus two hemispheres, each shifted to its end by
                // the parallel axis theorem.
                let length = 2.0 * half_height;
                let cylinder = PI * radius * radius * length * density;
                let hemisphere = 2.0 / 3.0 * PI * radius.powi(3) * density;
                let mass = cylinder + 2.0 * hemisphere;

                let along =
                    cylinder * radius * radius * 0.5 + 2.0 * hemisphere * 0.4 * radius * radius;
                let across = cylinder * (length * length / 12.0 + radius * radius * 0.25)
                    + 2.0
                        * hemisphere
                        * (0.4 * radius * radius
                            + half_height * half_height
                            + 0.375 * radius * length);
                MassProperties {
                    mass,
                    inertia: Vec3::new(across, along, across),
                }
            }
            // Infinite: only ever static.
            Shape::HalfSpace { .. } => MassProperties::ZERO,
        }
    }

    /// The point of the shape furthest along `direction`, in local space.
    ///
    /// Not used by the analytic collision routines, but it is what any future
    /// convex algorithm (GJK, MPR) would be built on.
    pub fn support(&self, direction: Vec3) -> Vec3 {
        match *self {
            Shape::Sphere { radius } => direction.normalized() * radius,
            Shape::Cuboid { half_extents } => Vec3::new(
                half_extents.x.copysign(direction.x),
                half_extents.y.copysign(direction.y),
                half_extents.z.copysign(direction.z),
            ),
            Shape::Capsule {
                half_height,
                radius,
            } => {
                let end = Vec3::new(0.0, half_height.copysign(direction.y), 0.0);
                end + direction.normalized() * radius
            }
            Shape::HalfSpace { normal } => normal * f32::INFINITY,
        }
    }

    /// Whether this shape can only ever be static.
    pub fn is_infinite(&self) -> bool {
        matches!(self, Shape::HalfSpace { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

    #[test]
    fn an_isometry_and_its_inverse_cancel() {
        let at = Isometry::new(Vec3::new(1.0, -2.0, 3.0), Quat::from_euler(0.4, 0.9, -0.2));
        let point = Vec3::new(0.5, 0.25, -1.5);
        let there_and_back = at.inverse_transform_point(at.transform_point(point));
        assert!((there_and_back - point).length() < 1e-5);

        let inverse = at.inverse();
        assert!((inverse.transform_point(at.transform_point(point)) - point).length() < 1e-5);
    }

    #[test]
    fn a_rotated_box_grows_its_bounds() {
        let shape = Shape::cuboid(Vec3::new(1.0, 0.5, 0.25));
        let axis_aligned = shape.aabb(&Isometry::IDENTITY);
        assert_eq!(axis_aligned.half_extents(), Vec3::new(1.0, 0.5, 0.25));

        // Turned 45 degrees about Z, the box's silhouette is wider.
        let turned = Isometry::new(Vec3::ZERO, Quat::from_axis_angle(Vec3::Z, FRAC_PI_4));
        let rotated = shape.aabb(&turned);
        let expected = (1.0 + 0.5) * FRAC_PI_4.cos();
        assert!(
            (rotated.half_extents().x - expected).abs() < 1e-5,
            "{rotated:?}"
        );
        assert_eq!(
            rotated.half_extents().z,
            0.25,
            "the untouched axis is unchanged"
        );
    }

    #[test]
    fn a_capsules_bounds_follow_its_axis() {
        let shape = Shape::capsule(1.0, 0.25);
        let upright = shape.aabb(&Isometry::IDENTITY);
        assert_eq!(upright.half_extents(), Vec3::new(0.25, 1.25, 0.25));

        // Laid on its side, the long axis is X.
        let lying = Isometry::new(Vec3::ZERO, Quat::from_axis_angle(Vec3::Z, FRAC_PI_2));
        let sideways = shape.aabb(&lying);
        assert!(
            (sideways.half_extents().x - 1.25).abs() < 1e-5,
            "{sideways:?}"
        );
        assert!((sideways.half_extents().y - 0.25).abs() < 1e-5);
    }

    #[test]
    fn aabbs_overlap_when_they_should() {
        let a = Aabb::new(Vec3::ZERO, Vec3::splat(1.0));
        assert!(a.overlaps(&Aabb::new(Vec3::splat(0.5), Vec3::splat(2.0))));
        assert!(!a.overlaps(&Aabb::new(Vec3::splat(1.1), Vec3::splat(2.0))));
        assert!(a.overlaps(&a.expanded(0.1)));
        assert!(a.contains(Vec3::splat(0.5)));
        assert!(!a.contains(Vec3::splat(-0.5)));
    }

    #[test]
    fn a_spheres_mass_and_inertia_match_the_textbook() {
        let props = Shape::sphere(2.0).mass_properties(1.0);
        let expected_mass = 4.0 / 3.0 * PI * 8.0;
        assert!((props.mass - expected_mass).abs() < 1e-3);
        // I = 2/5 m r²
        assert!((props.inertia.x - 0.4 * expected_mass * 4.0).abs() < 1e-2);
        assert_eq!(
            props.inertia.x, props.inertia.z,
            "a sphere spins alike on every axis"
        );
    }

    #[test]
    fn a_box_is_hardest_to_spin_about_its_short_axis() {
        // Long in X: spinning about X is easy, about Y and Z is not.
        let props = Shape::cuboid(Vec3::new(2.0, 0.25, 0.25)).mass_properties(1.0);
        assert!(props.inertia.x < props.inertia.y);
        assert!((props.inertia.y - props.inertia.z).abs() < 1e-5);

        // And the classic m/12 (w² + d²) for a cube of side 2.
        let cube = Shape::cuboid(Vec3::splat(1.0)).mass_properties(1.0);
        assert!((cube.mass - 8.0).abs() < 1e-5);
        assert!((cube.inertia.x - 8.0 * (4.0 + 4.0) / 12.0).abs() < 1e-4);
    }

    #[test]
    fn a_capsule_with_no_cylinder_is_a_sphere() {
        let capsule = Shape::capsule(0.0, 0.7).mass_properties(2.0);
        let sphere = Shape::sphere(0.7).mass_properties(2.0);
        assert!(
            (capsule.mass - sphere.mass).abs() < 1e-4,
            "{capsule:?} vs {sphere:?}"
        );
        assert!((capsule.inertia.y - sphere.inertia.y).abs() < 1e-4);
        assert!((capsule.inertia.x - sphere.inertia.x).abs() < 1e-4);
    }

    #[test]
    fn a_long_capsule_is_easier_to_spin_about_its_own_axis() {
        let props = Shape::capsule(2.0, 0.3).mass_properties(1.0);
        assert!(props.inertia.y < props.inertia.x * 0.2, "{props:?}");
    }

    #[test]
    fn support_points_reach_the_far_side_of_a_shape() {
        let cuboid = Shape::cuboid(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(
            cuboid.support(Vec3::new(1.0, -1.0, 1.0)),
            Vec3::new(1.0, -2.0, 3.0)
        );

        let sphere = Shape::sphere(2.0);
        assert!((sphere.support(Vec3::X * 5.0) - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-6);

        let capsule = Shape::capsule(1.0, 0.5);
        let top = capsule.support(Vec3::Y);
        assert!((top.y - 1.5).abs() < 1e-6, "{top:?}");
    }

    #[test]
    fn a_half_space_has_no_finite_mass_or_bounds() {
        let ground = Shape::ground();
        assert!(ground.is_infinite());
        assert_eq!(ground.mass_properties(1.0), MassProperties::ZERO);
        assert!(ground.aabb(&Isometry::IDENTITY).min.x.is_infinite());
    }
}
