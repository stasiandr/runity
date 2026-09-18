//! Finding a way across the grid: A* for one agent, flow fields for a crowd.
//!
//! Both work in integer milli-units rather than floats. Not for speed — for
//! order: a priority queue keyed on `f32` has no total order (NaN, and ties
//! that compare differently after the last bit of rounding), and two machines
//! that pop nodes in a different order find different paths through an open
//! field. Integers make ties break the same way everywhere, which is the
//! whole ballgame for a shared world.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use runity_math::Vec3;

use crate::nav::{NavGrid, BLOCKED};

/// Cost of one straight step across open ground.
const STRAIGHT: u32 = 1_000;

/// Cost of one diagonal step: `sqrt(2)`, rounded.
const DIAGONAL: u32 = 1_414;

/// How a search should behave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathSettings {
    /// Most cells the search may expand before giving up.
    ///
    /// A budget is not a nicety: without one, asking for a route to somewhere
    /// walled off makes a villager search the entire world, once per request,
    /// in the middle of a tick.
    pub budget: usize,
    /// Whether diagonal steps are allowed (they never cut a blocked corner).
    pub diagonals: bool,
    /// Whether to pull the staircase out of the finished route.
    pub smooth: bool,
    /// Heuristic weight in per-mille. 1000 is exact; higher searches far less
    /// and returns a slightly longer path, which for a hundred villagers
    /// crossing a field is usually the better trade.
    pub heuristic_weight: u32,
}

impl Default for PathSettings {
    fn default() -> Self {
        Self {
            budget: 20_000,
            diagonals: true,
            smooth: true,
            heuristic_weight: 1_000,
        }
    }
}

impl PathSettings {
    /// Exact, unsmoothed, unlimited enough for a test.
    pub const EXACT: PathSettings = PathSettings {
        budget: usize::MAX,
        diagonals: true,
        smooth: false,
        heuristic_weight: 1_000,
    };

    /// The same search with a different budget.
    pub fn with_budget(self, budget: usize) -> Self {
        Self { budget, ..self }
    }
}

/// A route through the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    /// Waypoints, starting at the first cell and ending at the goal.
    pub points: Vec<Vec3>,
    /// Total movement cost, in cells of open ground.
    pub cost: f32,
    /// How many cells the search had to look at — worth watching.
    pub expanded: usize,
}

impl Path {
    /// Number of waypoints.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Whether the path leads nowhere.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Where the path ends.
    pub fn destination(&self) -> Option<Vec3> {
        self.points.last().copied()
    }

    /// Straight-line length of the route as it will actually be walked.
    pub fn length(&self) -> f32 {
        self.points
            .windows(2)
            .map(|pair| crate::spatial::ground_distance(pair[0], pair[1]))
            .sum()
    }
}

/// Why a search did not produce a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathError {
    /// Start or goal is outside the grid.
    OffGrid,
    /// Start or goal stands on a blocked cell.
    Blocked,
    /// The goal is walled off from the start.
    Unreachable,
    /// The search ran out of budget. The caller decides whether to retry with
    /// more, or to have the agent do something else.
    Budget {
        /// Cells expanded before giving up.
        expanded: usize,
    },
}

/// Reusable search buffers.
///
/// Hold one per system rather than allocating inside the search: a village of
/// two hundred agents re-planning every few seconds would otherwise spend its
/// time in the allocator.
#[derive(Clone, Debug, Default)]
pub struct PathFinder {
    open: BinaryHeap<Reverse<(u32, u32)>>,
    /// Cost from the start, valid only where `stamp` matches `generation`.
    best: Vec<u32>,
    came_from: Vec<u32>,
    stamp: Vec<u32>,
    closed: Vec<u32>,
    generation: u32,
    scratch: Vec<Vec3>,
}

impl PathFinder {
    /// A finder with no buffers yet; they grow to fit the first grid it sees.
    pub fn new() -> Self {
        Self::default()
    }

    /// Find a route from `start` to `goal`.
    pub fn find(
        &mut self,
        grid: &NavGrid,
        start: Vec3,
        goal: Vec3,
        settings: &PathSettings,
    ) -> core::result::Result<Path, PathError> {
        let (Some(start_cell), Some(goal_cell)) = (grid.cell_at(start), grid.cell_at(goal)) else {
            return Err(PathError::OffGrid);
        };
        if !grid.walkable(start_cell.0, start_cell.1) || !grid.walkable(goal_cell.0, goal_cell.1) {
            return Err(PathError::Blocked);
        }

        self.prepare(grid.len());
        let width = grid.width();
        let start_index = (start_cell.1 * width + start_cell.0) as u32;
        let goal_index = (goal_cell.1 * width + goal_cell.0) as u32;

        self.best[start_index as usize] = 0;
        self.stamp[start_index as usize] = self.generation;
        self.came_from[start_index as usize] = start_index;
        let weight = settings.heuristic_weight.max(1);
        self.open.push(Reverse((
            heuristic(grid, start_index, goal_index) * weight / 1_000,
            start_index,
        )));

        let mut expanded = 0;
        while let Some(Reverse((_, current))) = self.open.pop() {
            if self.closed[current as usize] == self.generation {
                continue; // a stale entry left by a cheaper route to the same cell
            }
            self.closed[current as usize] = self.generation;
            expanded += 1;

            if current == goal_index {
                let cost = self.best[current as usize] as f32 / STRAIGHT as f32;
                let points =
                    self.rebuild(grid, start, goal, start_index, goal_index, settings.smooth);
                return Ok(Path {
                    points,
                    cost,
                    expanded,
                });
            }
            if expanded >= settings.budget {
                return Err(PathError::Budget { expanded });
            }

            let (x, y) = (current as usize % width, current as usize / width);
            for (dx, dy, step) in NEIGHBOURS {
                if !settings.diagonals && dx != 0 && dy != 0 {
                    continue;
                }
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || ny < 0 {
                    continue;
                }
                let (nx, ny) = (nx as usize, ny as usize);
                let cost = grid.cost(nx, ny);
                if cost == BLOCKED {
                    continue;
                }
                // A diagonal may not squeeze between two blocked corners: a
                // villager walking through the gap between two walls looks
                // exactly as wrong as it is.
                if dx != 0 && dy != 0 {
                    let side_a = grid.walkable((x as i32 + dx) as usize, y);
                    let side_b = grid.walkable(x, (y as i32 + dy) as usize);
                    if !(side_a && side_b) {
                        continue;
                    }
                }

                let neighbour = (ny * width + nx) as u32;
                if self.closed[neighbour as usize] == self.generation {
                    continue;
                }
                let tentative = self.best[current as usize] + step * u32::from(cost);
                let known = if self.stamp[neighbour as usize] == self.generation {
                    self.best[neighbour as usize]
                } else {
                    u32::MAX
                };
                if tentative >= known {
                    continue;
                }
                self.best[neighbour as usize] = tentative;
                self.came_from[neighbour as usize] = current;
                self.stamp[neighbour as usize] = self.generation;
                let estimate = heuristic(grid, neighbour, goal_index) * weight / 1_000;
                self.open.push(Reverse((tentative + estimate, neighbour)));
            }
        }

        Err(PathError::Unreachable)
    }

    fn prepare(&mut self, cells: usize) {
        if self.best.len() != cells {
            self.best = vec![u32::MAX; cells];
            self.came_from = vec![0; cells];
            self.stamp = vec![0; cells];
            self.closed = vec![0; cells];
            self.generation = 0;
        }
        self.open.clear();
        // A generation counter replaces clearing four arrays per search; it is
        // the difference between a search that costs what it explores and one
        // that costs the whole map.
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.stamp.iter_mut().for_each(|s| *s = 0);
            self.closed.iter_mut().for_each(|s| *s = 0);
            self.generation = 1;
        }
    }

    fn rebuild(
        &mut self,
        grid: &NavGrid,
        start: Vec3,
        goal: Vec3,
        start_index: u32,
        goal_index: u32,
        smooth: bool,
    ) -> Vec<Vec3> {
        self.scratch.clear();
        let mut current = goal_index;
        loop {
            let (x, y) = (
                current as usize % grid.width(),
                current as usize / grid.width(),
            );
            self.scratch.push(grid.cell_centre(x, y));
            if current == start_index {
                break;
            }
            current = self.came_from[current as usize];
        }
        self.scratch.reverse();

        // The real endpoints, not the cell centres they happen to sit in.
        if let Some(first) = self.scratch.first_mut() {
            *first = Vec3 {
                y: first.y,
                ..start
            };
        }
        if let Some(last) = self.scratch.last_mut() {
            *last = Vec3 { y: last.y, ..goal };
        }

        if !smooth || self.scratch.len() < 3 {
            return self.scratch.clone();
        }

        // String pulling: keep a waypoint only where the corner actually
        // turns, which is what separates a route from a staircase.
        let mut pulled = Vec::with_capacity(self.scratch.len());
        pulled.push(self.scratch[0]);
        let mut anchor = 0;
        for index in 2..self.scratch.len() {
            if !grid.line_of_sight(self.scratch[anchor], self.scratch[index]) {
                pulled.push(self.scratch[index - 1]);
                anchor = index - 1;
            }
        }
        pulled.push(
            *self
                .scratch
                .last()
                .expect("the path has at least one point"),
        );
        pulled
    }
}

/// Find one path with a throwaway finder — fine for a test or a one-off, but
/// hold a [`PathFinder`] in anything that runs every tick.
pub fn find_path(
    grid: &NavGrid,
    start: Vec3,
    goal: Vec3,
    settings: &PathSettings,
) -> core::result::Result<Path, PathError> {
    PathFinder::new().find(grid, start, goal, settings)
}

/// Octile distance: the exact cost of crossing open ground, so the search
/// never explores more than it has to and never returns a longer route than
/// it should.
fn heuristic(grid: &NavGrid, from: u32, to: u32) -> u32 {
    let width = grid.width();
    let (fx, fy) = (from as i32 % width as i32, from as i32 / width as i32);
    let (tx, ty) = (to as i32 % width as i32, to as i32 / width as i32);
    let (dx, dy) = ((fx - tx).unsigned_abs(), (fy - ty).unsigned_abs());
    let (low, high) = if dx < dy { (dx, dy) } else { (dy, dx) };
    DIAGONAL * low + STRAIGHT * (high - low)
}

/// Neighbour offsets and their step costs, in a fixed order so that equal-cost
/// routes always come out the same.
const NEIGHBOURS: [(i32, i32, u32); 8] = [
    (0, -1, STRAIGHT),
    (-1, 0, STRAIGHT),
    (1, 0, STRAIGHT),
    (0, 1, STRAIGHT),
    (-1, -1, DIAGONAL),
    (1, -1, DIAGONAL),
    (-1, 1, DIAGONAL),
    (1, 1, DIAGONAL),
];

/// Distance to the nearest goal from every cell, and the direction to walk.
///
/// When two hundred villagers head for the same granary, planning two hundred
/// paths is two hundred searches of the same map. One flood fill from the
/// granary answers all of them, and keeps answering as they move.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowField {
    width: usize,
    height: usize,
    /// Cost to the nearest goal, `u32::MAX` where there is no way through.
    distance: Vec<u32>,
}

impl FlowField {
    /// Flood the grid outward from every goal at once.
    pub fn build(grid: &NavGrid, goals: &[Vec3]) -> FlowField {
        let width = grid.width();
        let mut distance = vec![u32::MAX; grid.len()];
        let mut queue: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();

        for goal in goals {
            if let Some((x, y)) = grid.cell_at(*goal) {
                if !grid.walkable(x, y) {
                    continue;
                }
                let index = (y * width + x) as u32;
                if distance[index as usize] != 0 {
                    distance[index as usize] = 0;
                    queue.push(Reverse((0, index)));
                }
            }
        }

        while let Some(Reverse((cost, current))) = queue.pop() {
            if cost > distance[current as usize] {
                continue;
            }
            let (x, y) = (current as usize % width, current as usize / width);
            for (dx, dy, step) in NEIGHBOURS {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || ny < 0 {
                    continue;
                }
                let (nx, ny) = (nx as usize, ny as usize);
                let cell_cost = grid.cost(nx, ny);
                if cell_cost == BLOCKED {
                    continue;
                }
                if dx != 0 && dy != 0 {
                    let side_a = grid.walkable((x as i32 + dx) as usize, y);
                    let side_b = grid.walkable(x, (y as i32 + dy) as usize);
                    if !(side_a && side_b) {
                        continue;
                    }
                }
                let neighbour = ny * width + nx;
                let tentative = cost + step * u32::from(cell_cost);
                if tentative < distance[neighbour] {
                    distance[neighbour] = tentative;
                    queue.push(Reverse((tentative, neighbour as u32)));
                }
            }
        }

        FlowField {
            width,
            height: grid.height(),
            distance,
        }
    }

    /// Cost from a position to the nearest goal, in cells of open ground.
    pub fn distance_at(&self, grid: &NavGrid, position: Vec3) -> Option<f32> {
        let (x, y) = grid.cell_at(position)?;
        let value = *self.distance.get(y * self.width + x)?;
        (value != u32::MAX).then(|| value as f32 / STRAIGHT as f32)
    }

    /// Whether a goal can be reached from here at all.
    pub fn reachable(&self, grid: &NavGrid, position: Vec3) -> bool {
        self.distance_at(grid, position).is_some()
    }

    /// Which way to walk from here. `None` at a goal, or where nothing leads
    /// anywhere.
    pub fn direction_at(&self, grid: &NavGrid, position: Vec3) -> Option<Vec3> {
        let (x, y) = grid.cell_at(position)?;
        let here = *self.distance.get(y * self.width + x)?;
        if here == u32::MAX {
            return None;
        }
        if here == 0 {
            return None;
        }

        let (tx, ty) = self.best_neighbour(grid, x, y, here)?;
        let step = grid.cell_centre(tx, ty) - grid.cell_centre(x, y);
        Some(step.normalized())
    }

    /// The centre of the next cell on the way to the goal.
    ///
    /// Prefer this to [`FlowField::direction_at`] for anything that actually
    /// moves: a direction is a straight line, and a straight line from the
    /// middle of one cell can clip the corner of a wall on its way to the
    /// next. A point to walk towards cannot.
    pub fn next_step(&self, grid: &NavGrid, position: Vec3) -> Option<Vec3> {
        let (x, y) = grid.cell_at(position)?;
        let here = *self.distance.get(y * self.width + x)?;
        if here == u32::MAX || here == 0 {
            return None;
        }
        let (tx, ty) = self.best_neighbour(grid, x, y, here)?;
        Some(grid.cell_centre(tx, ty))
    }

    /// The neighbour that gets closest to a goal, obeying the same corner rule
    /// the flood fill used — otherwise the field would point through a gap the
    /// distances say does not exist.
    fn best_neighbour(
        &self,
        grid: &NavGrid,
        x: usize,
        y: usize,
        here: u32,
    ) -> Option<(usize, usize)> {
        let mut best = here;
        let mut target = None;
        for (dx, dy, _) in NEIGHBOURS {
            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
            if nx < 0 || ny < 0 || nx as usize >= self.width || ny as usize >= self.height {
                continue;
            }
            let (nx, ny) = (nx as usize, ny as usize);
            if !grid.walkable(nx, ny) {
                continue;
            }
            if dx != 0 && dy != 0 {
                let side_a = grid.walkable((x as i32 + dx) as usize, y);
                let side_b = grid.walkable(x, (y as i32 + dy) as usize);
                if !(side_a && side_b) {
                    continue;
                }
            }
            let value = self.distance[ny * self.width + nx];
            if value < best {
                best = value;
                target = Some((nx, ny));
            }
        }
        target
    }

    /// Cells in the field.
    pub fn len(&self) -> usize {
        self.distance.len()
    }

    /// Whether the field covers nothing.
    pub fn is_empty(&self) -> bool {
        self.distance.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::OPEN;
    use runity_math::{vec2, vec3, Rng, Vec2};

    fn open_grid(size: usize) -> NavGrid {
        NavGrid::new(Vec2::ZERO, 1.0, size, size)
    }

    /// An independent shortest-path cost, computed the slow obvious way.
    ///
    /// A* is only worth anything if it returns what exhaustive relaxation
    /// would, so the test compares against a different algorithm rather than
    /// against A* run twice.
    fn reference_cost(grid: &NavGrid, start: (usize, usize), goal: (usize, usize)) -> Option<u32> {
        let width = grid.width();
        let mut best = vec![u32::MAX; grid.len()];
        best[start.1 * width + start.0] = 0;
        // Relax until nothing changes: Bellman-Ford, with no priority queue
        // and no heuristic to get wrong.
        let mut changed = true;
        while changed {
            changed = false;
            for y in 0..grid.height() {
                for x in 0..width {
                    let here = best[y * width + x];
                    if here == u32::MAX {
                        continue;
                    }
                    for (dx, dy, step) in NEIGHBOURS {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        if nx < 0 || ny < 0 || nx as usize >= width || ny as usize >= grid.height()
                        {
                            continue;
                        }
                        let (nx, ny) = (nx as usize, ny as usize);
                        let cost = grid.cost(nx, ny);
                        if cost == BLOCKED {
                            continue;
                        }
                        if dx != 0 && dy != 0 {
                            let a = grid.walkable((x as i32 + dx) as usize, y);
                            let b = grid.walkable(x, (y as i32 + dy) as usize);
                            if !(a && b) {
                                continue;
                            }
                        }
                        let candidate = here + step * u32::from(cost);
                        if candidate < best[ny * width + nx] {
                            best[ny * width + nx] = candidate;
                            changed = true;
                        }
                    }
                }
            }
        }
        let value = best[goal.1 * width + goal.0];
        (value != u32::MAX).then_some(value)
    }

    #[test]
    fn a_straight_run_across_open_ground_is_a_straight_line() {
        let grid = open_grid(20);
        let start = grid.cell_centre(1, 10);
        let goal = grid.cell_centre(18, 10);
        let path = find_path(&grid, start, goal, &PathSettings::default()).unwrap();

        assert_eq!(path.points.first().copied(), Some(start));
        assert_eq!(path.points.last().copied(), Some(goal));
        assert_eq!(
            path.len(),
            2,
            "nothing is in the way, so there is nothing to turn around"
        );
        assert!((path.length() - 17.0).abs() < 1e-4);
    }

    #[test]
    fn a_wall_is_walked_around() {
        let mut grid = open_grid(20);
        // A wall with a gap at the bottom.
        for y in 0..17 {
            grid.block(10, y);
        }
        let start = grid.cell_centre(2, 2);
        let goal = grid.cell_centre(17, 2);
        let path = find_path(&grid, start, goal, &PathSettings::default()).unwrap();

        assert!(
            path.length() > 15.0,
            "going around costs more than going through"
        );
        // Every segment of the finished path has to be walkable, or smoothing
        // has quietly cut a corner through the wall.
        for pair in path.points.windows(2) {
            assert!(
                grid.line_of_sight(pair[0], pair[1]),
                "{:?} -> {:?}",
                pair[0],
                pair[1]
            );
        }
        assert!(
            path.points.iter().any(|p| p.z > 14.0),
            "the path dips to the gap"
        );
    }

    #[test]
    fn the_route_costs_what_an_independent_search_says_it_should() {
        let mut rng = Rng::named(1, "maze");
        for attempt in 0..20 {
            let mut grid = open_grid(24);
            for _ in 0..120 {
                let x = rng.below(24) as usize;
                let y = rng.below(24) as usize;
                grid.block(x, y);
            }
            let (start, goal) = ((0, 0), (23, 23));
            grid.set_cost(start.0, start.1, OPEN);
            grid.set_cost(goal.0, goal.1, OPEN);

            let from = grid.cell_centre(start.0, start.1);
            let to = grid.cell_centre(goal.0, goal.1);
            let found = find_path(&grid, from, to, &PathSettings::EXACT);
            match (found, reference_cost(&grid, start, goal)) {
                (Ok(path), Some(expected)) => {
                    let expected = expected as f32 / STRAIGHT as f32;
                    assert!(
                        (path.cost - expected).abs() < 1e-4,
                        "attempt {attempt}: A* said {}, exhaustive relaxation said {expected}",
                        path.cost
                    );
                }
                (Err(PathError::Unreachable), None) => {}
                (found, expected) => panic!("attempt {attempt}: {found:?} vs {expected:?}"),
            }
        }
    }

    #[test]
    fn expensive_ground_is_avoided_when_going_round_is_cheaper() {
        let mut grid = open_grid(20);
        // A band of deep mud across the middle.
        for x in 0..14 {
            for y in 9..12 {
                grid.set_cost(x, y, 9);
            }
        }
        let start = grid.cell_centre(2, 5);
        let goal = grid.cell_centre(2, 15);
        let path = find_path(&grid, start, goal, &PathSettings::EXACT).unwrap();

        // The direct line is ten cells; going round the mud is much further,
        // and still cheaper than wading three cells of cost nine.
        assert!(
            path.length() > 12.0,
            "it should detour, not wade: {}",
            path.length()
        );
        assert!(path.points.iter().any(|p| p.x > 13.0), "round the open end");

        // With the mud merely damp, the direct route wins again.
        for x in 0..14 {
            for y in 9..12 {
                grid.set_cost(x, y, 1);
            }
        }
        let direct = find_path(&grid, start, goal, &PathSettings::EXACT).unwrap();
        assert!((direct.length() - 10.0).abs() < 1e-3);
    }

    #[test]
    fn a_path_never_cuts_a_blocked_corner() {
        let mut grid = open_grid(8);
        grid.block(3, 4);
        grid.block(4, 3);
        let start = grid.cell_centre(3, 3);
        let goal = grid.cell_centre(4, 4);
        let path = find_path(&grid, start, goal, &PathSettings::EXACT).unwrap();
        // The two open cells are diagonal neighbours, but the only way across
        // is the long way round.
        assert!(
            path.length() > 2.0,
            "it squeezed through the corner: {}",
            path.length()
        );
    }

    #[test]
    fn four_way_movement_can_be_asked_for() {
        let grid = open_grid(10);
        let settings = PathSettings {
            diagonals: false,
            smooth: false,
            ..PathSettings::EXACT
        };
        let path = find_path(
            &grid,
            grid.cell_centre(1, 1),
            grid.cell_centre(4, 4),
            &settings,
        )
        .unwrap();
        // Manhattan, not octile: six steps rather than three diagonals.
        assert!((path.cost - 6.0).abs() < 1e-4, "{}", path.cost);
        for pair in path.points.windows(2) {
            let step = pair[1] - pair[0];
            assert!(
                step.x.abs() < 1e-4 || step.z.abs() < 1e-4,
                "a diagonal slipped in"
            );
        }
    }

    #[test]
    fn an_unreachable_goal_is_reported_rather_than_searched_forever() {
        let mut grid = open_grid(20);
        for y in 0..20 {
            grid.block(10, y);
        }
        let result = find_path(
            &grid,
            grid.cell_centre(2, 2),
            grid.cell_centre(17, 17),
            &PathSettings::EXACT,
        );
        assert_eq!(result, Err(PathError::Unreachable));
    }

    #[test]
    fn a_budget_stops_a_hopeless_search() {
        let mut grid = open_grid(60);
        for y in 0..60 {
            grid.block(30, y);
        }
        let settings = PathSettings::default().with_budget(50);
        let result = find_path(
            &grid,
            grid.cell_centre(2, 2),
            grid.cell_centre(50, 50),
            &settings,
        );
        match result {
            Err(PathError::Budget { expanded }) => assert!(expanded <= 51, "{expanded}"),
            other => panic!("expected the search to give up, got {other:?}"),
        }
    }

    #[test]
    fn bad_endpoints_are_told_apart() {
        let mut grid = open_grid(10);
        grid.block(5, 5);
        let open = grid.cell_centre(1, 1);

        assert_eq!(
            find_path(&grid, open, vec3(500.0, 0.0, 500.0), &PathSettings::EXACT),
            Err(PathError::OffGrid)
        );
        assert_eq!(
            find_path(&grid, open, grid.cell_centre(5, 5), &PathSettings::EXACT),
            Err(PathError::Blocked)
        );
        assert_eq!(
            find_path(&grid, grid.cell_centre(5, 5), open, &PathSettings::EXACT),
            Err(PathError::Blocked)
        );
    }

    #[test]
    fn a_path_to_where_you_already_are_is_trivial() {
        let grid = open_grid(10);
        let here = grid.cell_centre(4, 4);
        let path = find_path(&grid, here, here, &PathSettings::default()).unwrap();
        assert_eq!(path.len(), 1);
        assert_eq!(path.cost, 0.0);
    }

    #[test]
    fn the_same_query_always_returns_the_same_route() {
        // Ties are everywhere on open ground; if they break differently
        // between runs, two machines disagree about where a villager walked.
        let mut grid = open_grid(30);
        let mut rng = Rng::named(2, "ties");
        for _ in 0..80 {
            grid.block(rng.below(30) as usize, rng.below(30) as usize);
        }
        grid.set_cost(0, 0, OPEN);
        grid.set_cost(29, 29, OPEN);
        let (from, to) = (grid.cell_centre(0, 0), grid.cell_centre(29, 29));

        let first = find_path(&grid, from, to, &PathSettings::default());
        for _ in 0..5 {
            assert_eq!(find_path(&grid, from, to, &PathSettings::default()), first);
        }

        // And a reused finder, whose buffers still hold the last search,
        // agrees with a fresh one.
        let mut finder = PathFinder::new();
        let _ = finder.find(
            &grid,
            grid.cell_centre(5, 5),
            grid.cell_centre(20, 3),
            &PathSettings::default(),
        );
        assert_eq!(
            finder.find(&grid, from, to, &PathSettings::default()),
            first
        );
    }

    #[test]
    fn a_weighted_search_looks_at_less_and_still_arrives() {
        let mut grid = open_grid(60);
        for y in 10..50 {
            grid.block(30, y);
        }
        let (from, to) = (grid.cell_centre(2, 30), grid.cell_centre(57, 30));

        let exact = find_path(&grid, from, to, &PathSettings::EXACT).unwrap();
        let hurried = find_path(
            &grid,
            from,
            to,
            &PathSettings {
                heuristic_weight: 2_500,
                ..PathSettings::EXACT
            },
        )
        .unwrap();

        assert!(
            hurried.expanded < exact.expanded,
            "the whole point is to search less"
        );
        assert!(
            hurried.cost >= exact.cost - 1e-4,
            "and it cannot be shorter than optimal"
        );
        assert!(
            hurried.cost < exact.cost * 1.5,
            "but it should not wander either"
        );
    }

    #[test]
    fn smoothing_shortens_the_route_without_leaving_the_ground() {
        let mut grid = open_grid(40);
        for y in 0..25 {
            grid.block(20, y);
        }
        let (from, to) = (grid.cell_centre(3, 5), grid.cell_centre(36, 5));

        let staircase = find_path(&grid, from, to, &PathSettings::EXACT).unwrap();
        let pulled = find_path(&grid, from, to, &PathSettings::default()).unwrap();

        assert!(pulled.len() < staircase.len(), "a route, not a staircase");
        assert!(pulled.length() <= staircase.length() + 1e-3);
        for pair in pulled.points.windows(2) {
            assert!(grid.line_of_sight(pair[0], pair[1]));
        }
    }

    // ----------------------------------------------------------- flow field

    #[test]
    fn a_flow_field_leads_everyone_to_the_goal() {
        let mut grid = open_grid(30);
        for y in 0..24 {
            grid.block(15, y);
        }
        let goal = grid.cell_centre(28, 28);
        let field = FlowField::build(&grid, &[goal]);

        // Walk the field from several starting points; each has to arrive,
        // and must never find itself standing in a wall on the way.
        for start in [(1, 1), (5, 20), (14, 3), (20, 25), (29, 0)] {
            let mut position = grid.cell_centre(start.0, start.1);
            let mut steps = 0;
            while crate::spatial::ground_distance(position, goal) > 1.5 {
                let waypoint = field
                    .next_step(&grid, position)
                    .unwrap_or_else(|| panic!("stuck at {position:?} from {start:?}"));
                let step = (waypoint - position).normalized() * 0.5;
                position += step;
                let (cx, cy) = grid.cell_at(position).expect("still on the grid");
                assert!(grid.walkable(cx, cy), "walked into a wall at {position:?}");
                steps += 1;
                assert!(steps < 500, "walked in circles from {start:?}");
            }
        }
    }

    #[test]
    fn a_flow_field_prefers_the_nearer_of_two_goals() {
        let grid = open_grid(40);
        let near = grid.cell_centre(5, 5);
        let far = grid.cell_centre(35, 35);
        let field = FlowField::build(&grid, &[near, far]);

        let beside_the_near_goal = grid.cell_centre(8, 5);
        let direction = field.direction_at(&grid, beside_the_near_goal).unwrap();
        assert!(
            direction.x < 0.0,
            "it should head for the goal three cells away"
        );

        assert!(field.distance_at(&grid, beside_the_near_goal).unwrap() < 4.0);
        assert_eq!(field.distance_at(&grid, near), Some(0.0));
    }

    #[test]
    fn a_flow_field_knows_where_it_cannot_reach() {
        let mut grid = open_grid(20);
        // Seal a room off completely.
        for i in 0..6 {
            grid.block(5, i);
            grid.block(i, 5);
        }
        let goal = grid.cell_centre(15, 15);
        let field = FlowField::build(&grid, &[goal]);

        assert!(field.reachable(&grid, grid.cell_centre(10, 10)));
        assert!(
            !field.reachable(&grid, grid.cell_centre(2, 2)),
            "the sealed room is sealed"
        );
        assert_eq!(field.direction_at(&grid, grid.cell_centre(2, 2)), None);
        assert_eq!(
            field.direction_at(&grid, goal),
            None,
            "nowhere to go once you arrive"
        );
        assert_eq!(field.distance_at(&grid, vec3(-50.0, 0.0, 0.0)), None);
    }

    #[test]
    fn a_flow_field_agrees_with_a_path_about_distance() {
        let mut grid = open_grid(30);
        for y in 5..25 {
            grid.block(12, y);
        }
        let goal = grid.cell_centre(25, 15);
        let field = FlowField::build(&grid, &[goal]);

        for start in [(2, 2), (3, 26), (8, 15), (20, 20)] {
            let from = grid.cell_centre(start.0, start.1);
            let path = find_path(&grid, from, goal, &PathSettings::EXACT).unwrap();
            let flow = field.distance_at(&grid, from).unwrap();
            // Both are shortest-path costs over the same grid, so they must
            // agree; if they do not, one of the two is wrong about corners.
            assert!(
                (flow - path.cost).abs() < 1e-3,
                "flow {flow} vs path {}",
                path.cost
            );
        }
    }

    #[test]
    fn building_a_field_with_no_reachable_goal_is_harmless() {
        let grid = open_grid(10);
        let empty = FlowField::build(&grid, &[]);
        assert_eq!(empty.len(), grid.len());
        assert!(!empty.is_empty());
        assert!(!empty.reachable(&grid, grid.cell_centre(5, 5)));

        let off_grid = FlowField::build(&grid, &[vec3(900.0, 0.0, 900.0)]);
        assert!(!off_grid.reachable(&grid, grid.cell_centre(5, 5)));

        let mut blocked = open_grid(10);
        blocked.fill_box(vec2(0.0, 0.0), vec2(10.0, 10.0), BLOCKED);
        let sealed = FlowField::build(&blocked, &[blocked.cell_centre(5, 5)]);
        assert!(!sealed.reachable(&blocked, blocked.cell_centre(5, 5)));
    }
}
