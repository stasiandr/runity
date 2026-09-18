//! The ground as the AI sees it: a grid of cells that are walkable, slow, or
//! blocked.
//!
//! A navigation mesh would be tighter, but a grid is the right trade here: it
//! is built from whatever the world happens to contain (terrain height,
//! buildings, water), it can be edited the instant a villager puts a wall
//! somewhere, and it is trivially reproducible — which a mesh built by
//! floating-point polygon clipping is not.

use runity_math::{vec2, vec3, Vec2, Vec3};
use runity_serialize::{Deserialize, Reader, Result as SerializeResult, Serialize, Writer};

/// Cost of a cell nothing can enter.
pub const BLOCKED: u8 = 0;

/// Cost of ordinary open ground.
pub const OPEN: u8 = 1;

/// A rectangle of ground, one byte per cell.
///
/// The byte is a movement cost multiplier rather than a flag: mud, snow and a
/// steep slope are all "passable but I would rather not", and a path that
/// understands that looks far more deliberate than one that only knows walls.
#[derive(Clone, Debug, PartialEq)]
pub struct NavGrid {
    origin: Vec2,
    cell_size: f32,
    width: usize,
    height: usize,
    cost: Vec<u8>,
}

impl NavGrid {
    /// An empty grid of open ground, with cell `(0, 0)` starting at `origin`.
    pub fn new(origin: Vec2, cell_size: f32, width: usize, height: usize) -> Self {
        Self {
            origin,
            cell_size: if cell_size > 0.0 { cell_size } else { 1.0 },
            width,
            height,
            cost: vec![OPEN; width * height],
        }
    }

    /// A grid whose cells are costed by sampling the world at their centres.
    ///
    /// The usual source is the same noise the terrain is drawn from, so what
    /// the AI walks on and what the player sees cannot drift apart.
    pub fn from_fn(
        origin: Vec2,
        cell_size: f32,
        width: usize,
        height: usize,
        mut cost: impl FnMut(Vec3) -> u8,
    ) -> Self {
        let mut grid = Self::new(origin, cell_size, width, height);
        for y in 0..height {
            for x in 0..width {
                let centre = grid.cell_centre(x, y);
                grid.cost[y * width + x] = cost(centre);
            }
        }
        grid
    }

    /// Cells across.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Cells down.
    pub fn height(&self) -> usize {
        self.height
    }

    /// World units per cell.
    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    /// World position of the grid's corner.
    pub fn origin(&self) -> Vec2 {
        self.origin
    }

    /// Total number of cells.
    pub fn len(&self) -> usize {
        self.cost.len()
    }

    /// Whether the grid has no cells at all.
    pub fn is_empty(&self) -> bool {
        self.cost.is_empty()
    }

    /// Whether a cell coordinate is inside the grid.
    pub fn contains(&self, x: usize, y: usize) -> bool {
        x < self.width && y < self.height
    }

    /// Flat index of a cell, if it exists.
    pub fn index(&self, x: usize, y: usize) -> Option<usize> {
        self.contains(x, y).then(|| y * self.width + x)
    }

    /// Cell coordinates from a flat index.
    pub fn coords(&self, index: usize) -> (usize, usize) {
        (index % self.width.max(1), index / self.width.max(1))
    }

    /// Movement cost of a cell: [`BLOCKED`] for impassable, higher for slower.
    pub fn cost(&self, x: usize, y: usize) -> u8 {
        self.index(x, y).map_or(BLOCKED, |index| self.cost[index])
    }

    /// Cost by flat index.
    pub fn cost_at(&self, index: usize) -> u8 {
        self.cost.get(index).copied().unwrap_or(BLOCKED)
    }

    /// Whether anything can walk here.
    pub fn walkable(&self, x: usize, y: usize) -> bool {
        self.cost(x, y) != BLOCKED
    }

    /// Set a cell's cost.
    pub fn set_cost(&mut self, x: usize, y: usize, cost: u8) {
        if let Some(index) = self.index(x, y) {
            self.cost[index] = cost;
        }
    }

    /// Make a cell impassable.
    pub fn block(&mut self, x: usize, y: usize) {
        self.set_cost(x, y, BLOCKED);
    }

    /// Cost every cell a box on the ground covers.
    ///
    /// This is how a building becomes an obstacle: give it the footprint of
    /// its bounds and every path around the village updates at once. A box
    /// that ends exactly on a cell boundary does not claim the cell on the
    /// far side of it — otherwise every grid-aligned wall would be a cell
    /// thicker than it looks, which is the sort of thing nobody notices until
    /// a doorway will not fit.
    pub fn fill_box(&mut self, min: Vec2, max: Vec2, cost: u8) {
        let (min_x, min_y, max_x, max_y) = self.cell_range(min, max);
        for y in min_y.max(0)..=max_y.min(self.height as i32 - 1) {
            for x in min_x.max(0)..=max_x.min(self.width as i32 - 1) {
                if y < 0 || x < 0 {
                    continue;
                }
                self.set_cost(x as usize, y as usize, cost);
            }
        }
    }

    /// Cost every cell within `radius` of a point.
    pub fn fill_circle(&mut self, centre: Vec3, radius: f32, cost: u8) {
        let (min_x, min_y, max_x, max_y) = self.cell_range(
            vec2(centre.x - radius, centre.z - radius),
            vec2(centre.x + radius, centre.z + radius),
        );
        for y in min_y.max(0)..=max_y.min(self.height as i32 - 1) {
            for x in min_x.max(0)..=max_x.min(self.width as i32 - 1) {
                if x < 0 || y < 0 {
                    continue;
                }
                let (cx, cy) = (x as usize, y as usize);
                let point = self.cell_centre(cx, cy);
                let (dx, dz) = (point.x - centre.x, point.z - centre.z);
                if dx * dx + dz * dz <= radius * radius {
                    self.set_cost(cx, cy, cost);
                }
            }
        }
    }

    /// The cell a world position falls in, if it is on the grid.
    pub fn cell_at(&self, position: Vec3) -> Option<(usize, usize)> {
        let (x, y) = self.floor_cell(vec2(position.x, position.z));
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return None;
        }
        Some((x as usize, y as usize))
    }

    /// The world position at the centre of a cell.
    pub fn cell_centre(&self, x: usize, y: usize) -> Vec3 {
        vec3(
            self.origin.x + (x as f32 + 0.5) * self.cell_size,
            0.0,
            self.origin.y + (y as f32 + 0.5) * self.cell_size,
        )
    }

    /// The nearest walkable cell to `position`, searched outward in rings.
    ///
    /// Needed constantly in practice: a villager standing inside a freshly
    /// built wall, or a target picked from a click that landed on a rock, has
    /// to path from *somewhere*.
    pub fn nearest_walkable(&self, position: Vec3, max_rings: usize) -> Option<(usize, usize)> {
        let (x, y) = self.floor_cell(vec2(position.x, position.z));
        for ring in 0..=max_rings as i32 {
            let mut best: Option<(usize, usize)> = None;
            for dy in -ring..=ring {
                for dx in -ring..=ring {
                    // Only the edge of the ring is new.
                    if ring > 0 && dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let (cx, cy) = (x + dx, y + dy);
                    if cx < 0 || cy < 0 {
                        continue;
                    }
                    let (cx, cy) = (cx as usize, cy as usize);
                    if self.walkable(cx, cy) && best.is_none() {
                        best = Some((cx, cy));
                    }
                }
            }
            if let Some(cell) = best {
                return Some(cell);
            }
        }
        None
    }

    /// Whether a straight line between two points crosses only walkable cells.
    ///
    /// Used to straighten paths: a grid route is a staircase, and a villager
    /// who walks the staircase looks like a villager following a grid.
    pub fn line_of_sight(&self, from: Vec3, to: Vec3) -> bool {
        let (Some(start), Some(end)) = (self.cell_at(from), self.cell_at(to)) else {
            return false;
        };
        if !self.walkable(start.0, start.1) || !self.walkable(end.0, end.1) {
            return false;
        }

        // A supercover walk: unlike Bresenham it visits every cell the line
        // touches, so a diagonal cannot slip between two blocked corners.
        let (mut x, mut y) = (start.0 as i32, start.1 as i32);
        let (end_x, end_y) = (end.0 as i32, end.1 as i32);
        let (dx, dy) = ((end_x - x).abs(), (end_y - y).abs());
        let (step_x, step_y) = (
            if end_x > x { 1 } else { -1 },
            if end_y > y { 1 } else { -1 },
        );
        let mut error = dx - dy;

        while (x, y) != (end_x, end_y) {
            let double = error * 2;
            if double > -dy {
                error -= dy;
                x += step_x;
            } else if double < dx {
                error += dx;
                y += step_y;
            }
            if x < 0 || y < 0 || !self.walkable(x as usize, y as usize) {
                return false;
            }
        }
        true
    }

    /// How much of the grid can be walked on, as a fraction.
    pub fn open_fraction(&self) -> f32 {
        if self.cost.is_empty() {
            return 0.0;
        }
        let open = self.cost.iter().filter(|c| **c != BLOCKED).count();
        open as f32 / self.cost.len() as f32
    }

    /// Inclusive cell bounds covering a rectangle, with the far edge
    /// exclusive so a grid-aligned box covers exactly what it looks like.
    fn cell_range(&self, min: Vec2, max: Vec2) -> (i32, i32, i32, i32) {
        let low = vec2(min.x.min(max.x), min.y.min(max.y));
        let high = vec2(min.x.max(max.x), min.y.max(max.y));
        let (min_x, min_y) = self.floor_cell(low);
        let local = (high - self.origin) * (1.0 / self.cell_size);
        (
            min_x,
            min_y,
            local.x.ceil() as i32 - 1,
            local.y.ceil() as i32 - 1,
        )
    }

    fn floor_cell(&self, point: Vec2) -> (i32, i32) {
        let local = (point - self.origin) * (1.0 / self.cell_size);
        (local.x.floor() as i32, local.y.floor() as i32)
    }
}

impl Serialize for NavGrid {
    fn serialize(&self, writer: &mut Writer) {
        writer
            .write(&self.origin)
            .f32(self.cell_size)
            .write(&self.width)
            .write(&self.height)
            .bytes(&self.cost);
    }
}

impl Deserialize for NavGrid {
    fn deserialize(reader: &mut Reader) -> SerializeResult<Self> {
        let origin = reader.read()?;
        let cell_size = reader.f32()?;
        let width: usize = reader.read()?;
        let height: usize = reader.read()?;
        let cost = reader.bytes()?.to_vec();
        // A grid whose byte count disagrees with its dimensions would index
        // out of bounds on the first query; refuse it here instead.
        if cost.len() != width.saturating_mul(height) {
            return Err(runity_serialize::Error::InvalidValue {
                position: reader.position(),
                what: "NavGrid dimensions",
            });
        }
        Ok(NavGrid {
            origin,
            cell_size,
            width,
            height,
            cost,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_serialize::{from_bytes, to_bytes, Error};

    fn grid() -> NavGrid {
        NavGrid::new(vec2(-10.0, -10.0), 1.0, 20, 20)
    }

    #[test]
    fn a_position_and_its_cell_agree() {
        let grid = grid();
        let (x, y) = grid.cell_at(vec3(-9.5, 0.0, -9.5)).unwrap();
        assert_eq!((x, y), (0, 0));
        let centre = grid.cell_centre(0, 0);
        assert!((centre.x + 9.5).abs() < 1e-5 && (centre.z + 9.5).abs() < 1e-5);

        // And the centre of a cell maps back to that cell.
        for (cx, cy) in [(0, 0), (5, 7), (19, 19)] {
            assert_eq!(grid.cell_at(grid.cell_centre(cx, cy)), Some((cx, cy)));
        }
        assert_eq!(grid.cell_at(vec3(100.0, 0.0, 0.0)), None);
        assert_eq!(
            grid.cell_at(vec3(-100.0, 0.0, 0.0)),
            None,
            "before the origin is off the grid"
        );
    }

    #[test]
    fn everything_starts_walkable_and_can_be_blocked() {
        let mut grid = grid();
        assert!(grid.walkable(3, 3));
        assert_eq!(grid.open_fraction(), 1.0);

        grid.block(3, 3);
        assert!(!grid.walkable(3, 3));
        assert_eq!(grid.cost(3, 3), BLOCKED);
        assert!(grid.open_fraction() < 1.0);

        grid.set_cost(4, 4, 5);
        assert_eq!(grid.cost(4, 4), 5);
        assert!(grid.walkable(4, 4), "slow is not the same as blocked");

        // Out of bounds reads as blocked rather than panicking.
        assert_eq!(grid.cost(100, 100), BLOCKED);
        grid.block(100, 100);
    }

    #[test]
    fn a_box_blocks_every_cell_it_touches() {
        let mut grid = grid();
        grid.fill_box(vec2(-2.0, -2.0), vec2(2.0, 2.0), BLOCKED);
        // The box covers world -2..2, which is cells 8..11 with a 1m grid
        // starting at -10.
        for y in 8..=11 {
            for x in 8..=11 {
                assert!(
                    !grid.walkable(x, y),
                    "cell ({x}, {y}) should be inside the box"
                );
            }
        }
        assert!(grid.walkable(7, 8));
        assert!(grid.walkable(12, 12));
    }

    #[test]
    fn a_circle_blocks_a_circle() {
        let mut grid = grid();
        grid.fill_circle(vec3(0.0, 0.0, 0.0), 3.0, BLOCKED);
        assert!(!grid.walkable(10, 10), "the centre is inside");
        assert!(grid.walkable(14, 10), "four metres out is not");
        // Corners of the bounding box stay open, which is what makes it a
        // circle rather than a square.
        assert!(grid.walkable(12, 12));
    }

    #[test]
    fn the_nearest_walkable_cell_is_found_by_rings() {
        let mut grid = grid();
        grid.fill_box(vec2(-3.0, -3.0), vec2(3.0, 3.0), BLOCKED);

        let stuck = vec3(0.0, 0.0, 0.0);
        let (x, y) = grid
            .nearest_walkable(stuck, 10)
            .expect("somewhere outside the block");
        assert!(grid.walkable(x, y));
        let escape = grid.cell_centre(x, y);
        assert!(escape.x.abs() > 3.0 || escape.z.abs() > 3.0);

        // A grid with nothing open at all reports nothing.
        let mut sealed = NavGrid::new(Vec2::ZERO, 1.0, 4, 4);
        sealed.fill_box(vec2(-1.0, -1.0), vec2(5.0, 5.0), BLOCKED);
        assert_eq!(sealed.nearest_walkable(vec3(1.0, 0.0, 1.0), 6), None);
    }

    #[test]
    fn line_of_sight_stops_at_a_wall() {
        let mut grid = grid();
        assert!(grid.line_of_sight(vec3(-8.0, 0.0, 0.0), vec3(8.0, 0.0, 0.0)));

        // A wall down the middle, with no gap.
        for y in 0..20 {
            grid.block(10, y);
        }
        assert!(!grid.line_of_sight(vec3(-8.0, 0.0, 0.0), vec3(8.0, 0.0, 0.0)));
        assert!(grid.line_of_sight(vec3(-8.0, 0.0, 0.0), vec3(-2.0, 0.0, 4.0)));
        // Off the grid is not visible.
        assert!(!grid.line_of_sight(vec3(-8.0, 0.0, 0.0), vec3(80.0, 0.0, 0.0)));
    }

    #[test]
    fn a_diagonal_cannot_slip_between_two_blocked_corners() {
        // The classic grid bug: a line from one open cell to another that
        // passes exactly through the corner where two walls meet.
        let mut grid = NavGrid::new(Vec2::ZERO, 1.0, 4, 4);
        grid.block(1, 2);
        grid.block(2, 1);
        let from = grid.cell_centre(1, 1);
        let to = grid.cell_centre(2, 2);
        assert!(!grid.line_of_sight(from, to));
    }

    #[test]
    fn a_grid_can_be_costed_from_the_world_it_describes() {
        let grid = NavGrid::from_fn(vec2(0.0, 0.0), 2.0, 8, 8, |centre| {
            if centre.x > 8.0 {
                BLOCKED
            } else if centre.z > 8.0 {
                4
            } else {
                OPEN
            }
        });
        assert_eq!(grid.cost(0, 0), OPEN);
        assert_eq!(grid.cost(7, 0), BLOCKED, "past x = 8 is blocked");
        assert_eq!(grid.cost(0, 7), 4, "past z = 8 is slow");
    }

    #[test]
    fn a_grid_survives_a_save_and_refuses_a_broken_one() {
        let mut grid = grid();
        grid.fill_box(vec2(-2.0, -2.0), vec2(2.0, 2.0), BLOCKED);
        grid.set_cost(0, 0, 9);

        let bytes = to_bytes(&grid);
        assert_eq!(from_bytes::<NavGrid>(&bytes).unwrap(), grid);

        // A grid whose byte count disagrees with its size would index out of
        // bounds on the first query.
        let mut writer = Writer::new();
        writer
            .write(&Vec2::ZERO)
            .f32(1.0)
            .write(&4usize)
            .write(&4usize)
            .bytes(&[1, 1]);
        assert!(matches!(
            from_bytes::<NavGrid>(writer.as_bytes()),
            Err(Error::InvalidValue {
                what: "NavGrid dimensions",
                ..
            })
        ));
    }

    #[test]
    fn a_degenerate_grid_answers_without_panicking() {
        let grid = NavGrid::new(Vec2::ZERO, 0.0, 0, 0);
        assert!(grid.is_empty());
        assert_eq!(
            grid.cell_size(),
            1.0,
            "a zero cell size would divide by zero"
        );
        assert_eq!(grid.cell_at(Vec3::ZERO), None);
        assert_eq!(grid.open_fraction(), 0.0);
        assert!(!grid.line_of_sight(Vec3::ZERO, Vec3::ZERO));
    }
}
