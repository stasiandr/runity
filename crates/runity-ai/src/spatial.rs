//! "What is near this point?" — the query everything else is built on.
//!
//! Perception, separation between agents, picking up resources, deciding
//! which region a server owns: all of them are the same question, asked
//! thousands of times a tick. Asking it by scanning every entity is quadratic
//! and shows up as a frame-rate cliff exactly when the village gets
//! interesting.
//!
//! A uniform hash grid answers it in roughly constant time and, unlike a tree,
//! costs nothing to rebuild — which matters because in a simulation almost
//! everything moves every tick.

use std::collections::HashMap;

use runity_math::{Vec2, Vec3};

/// A hashed grid of item ids over the ground plane.
///
/// Height is deliberately ignored: a settlement spreads out, not up, and a
/// third axis would triple the cells scanned for nothing. Filter by height
/// after the query if it matters.
#[derive(Clone, Debug)]
pub struct SpatialGrid {
    cell_size: f32,
    inverse_cell_size: f32,
    cells: HashMap<(i32, i32), Vec<u32>>,
    count: usize,
}

impl SpatialGrid {
    /// A grid whose cells are `cell_size` across.
    ///
    /// Pick it close to the radius you query most: much smaller and a query
    /// scans many cells, much larger and every cell holds too much.
    pub fn new(cell_size: f32) -> Self {
        let size = if cell_size > 0.0 { cell_size } else { 1.0 };
        Self {
            cell_size: size,
            inverse_cell_size: 1.0 / size,
            cells: HashMap::new(),
            count: 0,
        }
    }

    /// Cell width in world units.
    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    /// How many items are in the grid.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the grid holds nothing.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Drop everything, keeping the allocated cells for the next rebuild.
    pub fn clear(&mut self) {
        for bucket in self.cells.values_mut() {
            bucket.clear();
        }
        self.count = 0;
    }

    /// File an item under its position.
    pub fn insert(&mut self, id: u32, position: Vec3) {
        let key = self.key(position);
        self.cells.entry(key).or_default().push(id);
        self.count += 1;
    }

    /// Rebuild from scratch.
    ///
    /// Cheaper than moving items between cells one at a time, and it cannot
    /// leave a stale entry behind — which a moving-agent simulation otherwise
    /// does within an hour of being written.
    pub fn rebuild(&mut self, items: impl IntoIterator<Item = (u32, Vec3)>) {
        self.clear();
        for (id, position) in items {
            self.insert(id, position);
        }
    }

    /// Append every item within `radius` of `centre` to `out`.
    ///
    /// Results come back in a fixed order — cells are scanned in a fixed
    /// nested loop, items within a cell in insertion order — so a simulation
    /// that iterates them stays reproducible.
    pub fn query_radius(&self, centre: Vec3, radius: f32, out: &mut Vec<u32>) {
        out.clear();
        if radius <= 0.0 {
            return;
        }
        let (min_x, min_y) = self.cell(centre.x - radius, centre.z - radius);
        let (max_x, max_y) = self.cell(centre.x + radius, centre.z + radius);
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                if let Some(bucket) = self.cells.get(&(x, y)) {
                    out.extend_from_slice(bucket);
                }
            }
        }
    }

    /// Same, into a fresh vector.
    pub fn within(&self, centre: Vec3, radius: f32) -> Vec<u32> {
        let mut out = Vec::new();
        self.query_radius(centre, radius, &mut out);
        out
    }

    /// Every item in the cells covering an axis-aligned box on the ground.
    pub fn query_box(&self, min: Vec2, max: Vec2, out: &mut Vec<u32>) {
        out.clear();
        let (min_x, min_y) = self.cell(min.x.min(max.x), min.y.min(max.y));
        let (max_x, max_y) = self.cell(min.x.max(max.x), min.y.max(max.y));
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                if let Some(bucket) = self.cells.get(&(x, y)) {
                    out.extend_from_slice(bucket);
                }
            }
        }
    }

    /// The nearest item to `centre`, by the positions the lookup provides.
    ///
    /// The grid stores only ids, so distances are resolved through `position`
    /// — usually a component lookup. Ties go to the lower id, so two things
    /// in the same place always resolve the same way.
    pub fn nearest(
        &self,
        centre: Vec3,
        radius: f32,
        position: impl Fn(u32) -> Option<Vec3>,
    ) -> Option<(u32, f32)> {
        let mut candidates = Vec::new();
        self.query_radius(centre, radius, &mut candidates);
        let mut best: Option<(u32, f32)> = None;
        for id in candidates {
            let Some(point) = position(id) else {
                continue;
            };
            let distance = ground_distance(point, centre);
            if distance > radius {
                continue;
            }
            match best {
                Some((best_id, best_distance))
                    if best_distance < distance || (best_distance == distance && best_id <= id) => {
                }
                _ => best = Some((id, distance)),
            }
        }
        best
    }

    /// How many items sit in the same cell as `position` — a cheap crowding
    /// measure for AI that should avoid a scrum.
    pub fn density_at(&self, position: Vec3) -> usize {
        self.cells.get(&self.key(position)).map_or(0, Vec::len)
    }

    fn key(&self, position: Vec3) -> (i32, i32) {
        self.cell(position.x, position.z)
    }

    fn cell(&self, x: f32, z: f32) -> (i32, i32) {
        (
            (x * self.inverse_cell_size).floor() as i32,
            (z * self.inverse_cell_size).floor() as i32,
        )
    }
}

/// Distance on the ground plane, ignoring height.
#[inline]
pub fn ground_distance(a: Vec3, b: Vec3) -> f32 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_math::{vec2, vec3, Rng};

    fn grid_of(points: &[(u32, Vec3)]) -> SpatialGrid {
        let mut grid = SpatialGrid::new(4.0);
        grid.rebuild(points.iter().copied());
        grid
    }

    #[test]
    fn a_query_finds_everything_within_the_radius() {
        let points = [
            (0, vec3(0.0, 0.0, 0.0)),
            (1, vec3(3.0, 0.0, 0.0)),
            (2, vec3(0.0, 0.0, 9.0)),
            (3, vec3(-2.0, 5.0, -2.0)),
        ];
        let grid = grid_of(&points);

        let mut found = grid.within(vec3(0.0, 0.0, 0.0), 5.0);
        found.sort_unstable();
        // The grid answers by cell, so it may include a near miss; what it
        // must never do is leave out something that is genuinely inside.
        assert!(found.contains(&0) && found.contains(&1) && found.contains(&3));
        assert!(!found.contains(&2), "nine metres away is not within five");
    }

    #[test]
    fn a_query_never_misses_anything_a_brute_force_scan_finds() {
        // The property that matters: the grid may be generous, never mean.
        let mut rng = Rng::named(1, "spatial");
        let points: Vec<(u32, Vec3)> = (0..500)
            .map(|i| (i, vec3(rng.range(-60.0, 60.0), 0.0, rng.range(-60.0, 60.0))))
            .collect();
        let grid = grid_of(&points);

        for _ in 0..200 {
            let centre = vec3(rng.range(-60.0, 60.0), 0.0, rng.range(-60.0, 60.0));
            let radius = rng.range(0.5, 12.0);
            let found = grid.within(centre, radius);
            for (id, position) in &points {
                if ground_distance(*position, centre) <= radius {
                    assert!(found.contains(id), "missed {id} at {position:?}");
                }
            }
        }
    }

    #[test]
    fn results_come_back_in_the_same_order_every_time() {
        let mut rng = Rng::named(2, "order");
        let points: Vec<(u32, Vec3)> = (0..200)
            .map(|i| (i, vec3(rng.range(-30.0, 30.0), 0.0, rng.range(-30.0, 30.0))))
            .collect();
        let grid = grid_of(&points);
        let first = grid.within(vec3(0.0, 0.0, 0.0), 15.0);
        for _ in 0..5 {
            assert_eq!(grid.within(vec3(0.0, 0.0, 0.0), 15.0), first);
        }
        // And a grid built the same way answers the same way.
        assert_eq!(grid_of(&points).within(vec3(0.0, 0.0, 0.0), 15.0), first);
    }

    #[test]
    fn nearest_picks_the_closest_and_breaks_ties_by_id() {
        let points = [
            (7, vec3(2.0, 0.0, 0.0)),
            (3, vec3(5.0, 0.0, 0.0)),
            (9, vec3(2.0, 0.0, 0.0)),
        ];
        let grid = grid_of(&points);
        let lookup = |id: u32| points.iter().find(|(i, _)| *i == id).map(|(_, p)| *p);

        let (id, distance) = grid.nearest(vec3(0.0, 0.0, 0.0), 10.0, lookup).unwrap();
        assert_eq!(id, 7, "two things in one place resolve to the lower id");
        assert!((distance - 2.0).abs() < 1e-5);

        assert!(grid.nearest(vec3(100.0, 0.0, 0.0), 3.0, lookup).is_none());
        // An id the lookup does not know is skipped rather than fatal.
        assert_eq!(grid.nearest(vec3(0.0, 0.0, 0.0), 10.0, |_| None), None);
    }

    #[test]
    fn a_box_query_covers_its_corners() {
        let points = [
            (0, vec3(-5.0, 0.0, -5.0)),
            (1, vec3(5.0, 0.0, 5.0)),
            (2, vec3(50.0, 0.0, 50.0)),
        ];
        let grid = grid_of(&points);
        let mut found = Vec::new();
        grid.query_box(vec2(-6.0, -6.0), vec2(6.0, 6.0), &mut found);
        found.sort_unstable();
        assert!(found.contains(&0) && found.contains(&1));
        assert!(!found.contains(&2));
    }

    #[test]
    fn rebuilding_leaves_nothing_stale_behind() {
        let mut grid = SpatialGrid::new(2.0);
        grid.insert(1, vec3(0.0, 0.0, 0.0));
        assert_eq!(grid.len(), 1);

        grid.rebuild([(1, vec3(40.0, 0.0, 40.0))]);
        assert_eq!(grid.len(), 1);
        assert!(
            grid.within(vec3(0.0, 0.0, 0.0), 3.0).is_empty(),
            "the old cell is empty"
        );
        assert_eq!(grid.within(vec3(40.0, 0.0, 40.0), 3.0), vec![1]);

        grid.clear();
        assert!(grid.is_empty());
        assert!(grid.within(vec3(40.0, 0.0, 40.0), 3.0).is_empty());
    }

    #[test]
    fn negative_coordinates_land_in_their_own_cells() {
        // `as i32` truncation would fold -0.5 and 0.5 into the same cell and
        // quietly halve the world.
        let grid = grid_of(&[(0, vec3(-0.5, 0.0, -0.5)), (1, vec3(0.5, 0.0, 0.5))]);
        assert_eq!(grid.density_at(vec3(-0.5, 0.0, -0.5)), 1);
        assert_eq!(grid.density_at(vec3(0.5, 0.0, 0.5)), 1);
    }

    #[test]
    fn density_reports_the_local_crowd() {
        let mut grid = SpatialGrid::new(5.0);
        for id in 0..6 {
            grid.insert(id, vec3(1.0, 0.0, 1.0));
        }
        assert_eq!(grid.density_at(vec3(2.0, 0.0, 2.0)), 6);
        assert_eq!(grid.density_at(vec3(90.0, 0.0, 90.0)), 0);
    }

    #[test]
    fn a_degenerate_cell_size_does_not_divide_by_zero() {
        let grid = SpatialGrid::new(0.0);
        assert_eq!(grid.cell_size(), 1.0);
        assert!(grid.within(Vec3::ZERO, 0.0).is_empty());
    }
}
