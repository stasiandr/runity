//! The static half of the simulation: things that stand still and are in the
//! way — trees, walls, a felled log.

use crate::transform::Transform;
use crate::world::{Entity, World};
use runity_math::{Vec2, Vec3};
use std::collections::HashMap;

/// A shape a body cannot walk through, attached to an entity as a component.
///
/// A blocker has no position of its own: it sits at the
/// [`Transform`] of the entity carrying it, and `top` is measured **up from
/// that transform's `y`** — so a 6 m tree at ground level is
/// `Cylinder { radius: 0.35, top: 6.0 }`, and the same tree once felled is a
/// `Box` with `top: 0.3`.
///
/// Two shapes is the whole vocabulary. Trees and barrels are round; walls,
/// crates and logs are rectangles that need a facing. Anything else is built
/// out of several of these.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Blocker {
    /// A circle in XZ, extruded upwards.
    Cylinder { radius: f32, top: f32 },
    /// A rectangle in XZ, extruded upwards. `half` is the half-extent along
    /// `facing` (`half.x`) and across it (`half.y`); `facing` is a horizontal
    /// direction, normalized when the grid is built.
    Box { half: Vec2, facing: Vec2, top: f32 },
}

impl Blocker {
    /// Height of the shape's top surface, relative to its own base.
    #[inline]
    pub fn top(&self) -> f32 {
        match *self {
            Blocker::Cylinder { top, .. } => top,
            Blocker::Box { top, .. } => top,
        }
    }

    /// Half the width of the smallest axis-aligned box that contains the
    /// footprint — what the grid needs to know where to file the shape.
    #[inline]
    fn footprint_half(&self) -> Vec2 {
        match *self {
            Blocker::Cylinder { radius, .. } => Vec2::splat(radius),
            Blocker::Box { half, facing, .. } => {
                let f = facing.normalized();
                let r = Vec2::new(-f.y, f.x);
                Vec2::new(
                    (f.x * half.x).abs() + (r.x * half.y).abs(),
                    (f.y * half.x).abs() + (r.y * half.y).abs(),
                )
            }
        }
    }

    /// Resolve a circle of `radius` centred at `point` against this footprint,
    /// whose own centre is at `origin`.
    ///
    /// Returns the unit direction the circle has to move in and how far, or
    /// `None` when the two do not overlap at all. The normal is what makes
    /// contact a slide rather than a stop: the caller keeps the part of its
    /// velocity perpendicular to it.
    fn resolve(&self, origin: Vec2, point: Vec2, radius: f32) -> Option<(Vec2, f32)> {
        match *self {
            Blocker::Cylinder {
                radius: own_radius, ..
            } => {
                let delta = point - origin;
                let reach = radius + own_radius;
                let distance = delta.length();
                if distance >= reach {
                    return None;
                }
                // Dead centre has no direction to escape along; +X is as good
                // as any and keeps the result finite.
                let normal = if distance > 1e-6 {
                    delta * (1.0 / distance)
                } else {
                    Vec2::new(1.0, 0.0)
                };
                Some((normal, reach - distance))
            }
            Blocker::Box { half, facing, .. } => {
                let f = facing.normalized();
                let r = Vec2::new(-f.y, f.x);
                let delta = point - origin;
                let local = Vec2::new(delta.dot(f), delta.dot(r));
                let clamped = Vec2::new(
                    local.x.clamp(-half.x, half.x),
                    local.y.clamp(-half.y, half.y),
                );
                let out = local - clamped;
                let distance = out.length();
                if distance > 1e-6 {
                    if distance >= radius {
                        return None;
                    }
                    let n = out * (1.0 / distance);
                    Some((f * n.x + r * n.y, radius - distance))
                } else {
                    // The centre is inside the rectangle: leave by the nearest
                    // face, which is the shallowest way out.
                    let dx = half.x - local.x.abs();
                    let dy = half.y - local.y.abs();
                    let (n, depth) = if dx <= dy {
                        (Vec2::new(local.x.signum(), 0.0), dx + radius)
                    } else {
                        (Vec2::new(0.0, local.y.signum()), dy + radius)
                    };
                    Some((f * n.x + r * n.y, depth))
                }
            }
        }
    }
}

/// One blocker placed in the world, as the grid stores it.
#[derive(Debug, Clone, Copy)]
pub struct StaticShape {
    /// The entity the blocker is a component of.
    pub entity: Entity,
    /// The owning transform's position — the blocker's base.
    pub base: Vec3,
    pub blocker: Blocker,
}

impl StaticShape {
    /// World height of the shape's top surface.
    #[inline]
    pub fn top(&self) -> f32 {
        self.base.y + self.blocker.top()
    }

    /// See [`Blocker::resolve`]; `point` is the body's axis in XZ.
    #[inline]
    pub fn resolve(&self, point: Vec2, radius: f32) -> Option<(Vec2, f32)> {
        self.blocker
            .resolve(Vec2::new(self.base.x, self.base.z), point, radius)
    }

    /// Whether a circle of `radius` at `point` sits over this footprint at all.
    #[inline]
    pub fn covers(&self, point: Vec2, radius: f32) -> bool {
        self.resolve(point, radius).is_some()
    }
}

/// A uniform grid over the XZ plane holding every [`Blocker`] in the world.
///
/// The only spatial index in the engine, and only statics go in it. Bodies do
/// not: twelve of them is sixty-six pairs, which is cheaper to test outright
/// than to bucket and rebuild every tick. Trees and walls are the opposite —
/// thousands of them, and they move about twice an hour.
#[derive(Debug, Clone)]
pub struct StaticGrid {
    cell: f32,
    shapes: Vec<StaticShape>,
    buckets: HashMap<(i32, i32), Vec<u32>>,
}

impl StaticGrid {
    /// An empty grid with `cell`-metre cells.
    pub fn new(cell: f32) -> Self {
        assert!(cell > 0.0, "a grid cell must have a positive size");
        Self {
            cell,
            shapes: Vec::new(),
            buckets: HashMap::new(),
        }
    }

    pub fn cell_size(&self) -> f32 {
        self.cell
    }

    pub fn shapes(&self) -> &[StaticShape] {
        &self.shapes
    }

    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    #[inline]
    fn coord(&self, v: f32) -> i32 {
        (v / self.cell).floor() as i32
    }

    /// Refill from every entity that carries both a [`Blocker`] and a
    /// [`Transform`]. Entities are visited in slot order, so two worlds built
    /// the same way index the same way.
    pub fn rebuild(&mut self, world: &World) {
        self.shapes.clear();
        self.buckets.clear();
        for entity in world.entities_with::<Blocker>() {
            let Some(transform) = world.get::<Transform>(entity) else {
                continue;
            };
            let base = transform.position;
            let blocker = *world
                .get::<Blocker>(entity)
                .expect("entities_with only yields entities that have one");
            self.insert(StaticShape {
                entity,
                base,
                blocker,
            });
        }
    }

    /// File one shape into every cell its footprint touches.
    pub fn insert(&mut self, shape: StaticShape) {
        let index = self.shapes.len() as u32;
        let half = shape.blocker.footprint_half();
        let (min_x, max_x) = (
            self.coord(shape.base.x - half.x),
            self.coord(shape.base.x + half.x),
        );
        let (min_z, max_z) = (
            self.coord(shape.base.z - half.y),
            self.coord(shape.base.z + half.y),
        );
        for cz in min_z..=max_z {
            for cx in min_x..=max_x {
                self.buckets.entry((cx, cz)).or_default().push(index);
            }
        }
        self.shapes.push(shape);
    }

    /// Every shape in the nine cells around `(x, z)`, in a fixed order.
    ///
    /// A body is 0.6 m across and a cell is four metres, so one ring of
    /// neighbours is more reach than a tick of movement can ever need — the
    /// nine are there so a body standing on a cell edge still sees what is on
    /// the other side of it.
    pub fn query(&self, x: f32, z: f32, out: &mut Vec<u32>) {
        out.clear();
        let (cx, cz) = (self.coord(x), self.coord(z));
        for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(bucket) = self.buckets.get(&(cx + dx, cz + dz)) {
                    out.extend_from_slice(bucket);
                }
            }
        }
        // A shape spanning several of the nine cells is listed several times,
        // and `HashMap` hands the cells back in whatever order it likes.
        out.sort_unstable();
        out.dedup();
    }

    #[inline]
    pub fn shape(&self, index: u32) -> &StaticShape {
        &self.shapes[index as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These tests only exercise geometry, so any valid handle will do.
    fn an_entity() -> Entity {
        World::new().spawn()
    }

    fn cylinder(x: f32, z: f32, radius: f32, top: f32) -> StaticShape {
        StaticShape {
            entity: an_entity(),
            base: Vec3::new(x, 0.0, z),
            blocker: Blocker::Cylinder { radius, top },
        }
    }

    #[test]
    fn a_circle_outside_a_cylinder_does_not_resolve() {
        let shape = cylinder(0.0, 0.0, 1.0, 2.0);
        assert!(shape.resolve(Vec2::new(2.0, 0.0), 0.3).is_none());
    }

    #[test]
    fn a_circle_inside_a_cylinder_is_pushed_straight_out() {
        let shape = cylinder(0.0, 0.0, 1.0, 2.0);
        let (normal, depth) = shape.resolve(Vec2::new(0.9, 0.0), 0.3).unwrap();
        assert!((normal - Vec2::new(1.0, 0.0)).length() < 1e-5, "{normal:?}");
        assert!((depth - 0.4).abs() < 1e-5, "{depth}");
    }

    #[test]
    fn a_circle_exactly_on_a_cylinder_axis_still_gets_a_direction() {
        let shape = cylinder(0.0, 0.0, 1.0, 2.0);
        let (normal, depth) = shape.resolve(Vec2::ZERO, 0.3).unwrap();
        assert!((normal.length() - 1.0).abs() < 1e-5);
        assert!((depth - 1.3).abs() < 1e-5);
    }

    #[test]
    fn a_box_resolves_along_its_own_axes() {
        let shape = StaticShape {
            entity: an_entity(),
            base: Vec3::ZERO,
            blocker: Blocker::Box {
                half: Vec2::new(2.0, 0.5),
                // Facing +Z, so the long axis runs along Z and the short one
                // along X.
                facing: Vec2::new(0.0, 1.0),
                top: 0.3,
            },
        };
        // Approaching the long side from +X.
        let (normal, depth) = shape.resolve(Vec2::new(0.6, 0.0), 0.3).unwrap();
        assert!((normal - Vec2::new(1.0, 0.0)).length() < 1e-5, "{normal:?}");
        assert!((depth - 0.2).abs() < 1e-5, "{depth}");
        // Off the end of the long axis: the rectangle stops at z = 2.0, so a
        // 0.3 m circle still catches it at 2.2 and misses it at 2.4.
        let (normal, depth) = shape.resolve(Vec2::new(0.0, 2.2), 0.3).unwrap();
        assert!((normal - Vec2::new(0.0, 1.0)).length() < 1e-5, "{normal:?}");
        assert!((depth - 0.1).abs() < 1e-5, "{depth}");
        assert!(shape.resolve(Vec2::new(0.0, 2.4), 0.3).is_none());
    }

    #[test]
    fn a_box_centre_leaves_by_its_nearest_face() {
        let shape = StaticShape {
            entity: an_entity(),
            base: Vec3::ZERO,
            blocker: Blocker::Box {
                half: Vec2::new(2.0, 0.5),
                facing: Vec2::new(1.0, 0.0),
                top: 0.3,
            },
        };
        let (normal, depth) = shape.resolve(Vec2::new(0.0, 0.1), 0.3).unwrap();
        assert!((normal - Vec2::new(0.0, 1.0)).length() < 1e-5, "{normal:?}");
        assert!((depth - 0.7).abs() < 1e-5, "{depth}");
    }

    #[test]
    fn a_rotated_box_footprint_files_into_every_cell_it_touches() {
        let mut grid = StaticGrid::new(4.0);
        grid.insert(StaticShape {
            entity: an_entity(),
            base: Vec3::new(3.9, 0.0, 3.9),
            blocker: Blocker::Box {
                half: Vec2::new(1.0, 1.0),
                facing: Vec2::new(1.0, 1.0),
                top: 0.3,
            },
        });
        // The footprint straddles the (0,0)/(1,1) cell corner.
        let mut found = Vec::new();
        grid.query(5.0, 5.0, &mut found);
        assert_eq!(found, vec![0]);
        grid.query(1.0, 1.0, &mut found);
        assert_eq!(found, vec![0]);
    }

    #[test]
    fn a_query_lists_each_shape_once_and_in_a_fixed_order() {
        let mut grid = StaticGrid::new(4.0);
        for i in 0..6 {
            grid.insert(cylinder(i as f32 * 0.5, 0.0, 0.4, 3.0));
        }
        let mut found = Vec::new();
        grid.query(1.0, 0.0, &mut found);
        assert_eq!(found, vec![0, 1, 2, 3, 4, 5]);
        let mut again = Vec::new();
        grid.query(1.0, 0.0, &mut again);
        assert_eq!(found, again);
    }

    #[test]
    fn a_query_far_from_everything_is_empty() {
        let mut grid = StaticGrid::new(4.0);
        grid.insert(cylinder(0.0, 0.0, 0.4, 3.0));
        let mut found = Vec::new();
        grid.query(100.0, 100.0, &mut found);
        assert!(found.is_empty());
    }

    #[test]
    fn rebuild_reads_blockers_and_their_owners_transforms() {
        let mut world = World::new();
        let tree = world.spawn();
        world.insert(tree, Transform::from_position(Vec3::new(7.0, 1.0, -2.0)));
        world.insert(
            tree,
            Blocker::Cylinder {
                radius: 0.4,
                top: 6.0,
            },
        );
        // A blocker with no transform has nowhere to be, so it is skipped.
        let homeless = world.spawn();
        world.insert(
            homeless,
            Blocker::Cylinder {
                radius: 1.0,
                top: 1.0,
            },
        );

        let mut grid = StaticGrid::new(4.0);
        grid.rebuild(&world);
        assert_eq!(grid.len(), 1);
        assert_eq!(grid.shape(0).entity, tree);
        assert_eq!(grid.shape(0).top(), 7.0, "top is measured from the base");

        let mut found = Vec::new();
        grid.query(7.0, -2.0, &mut found);
        assert_eq!(found, vec![0]);
    }
}
