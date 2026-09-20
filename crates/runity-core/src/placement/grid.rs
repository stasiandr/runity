//! [`PlacementGrid`]: where a stake for a given deficit belongs.

use runity_math::{Vec2, Vec3};

use crate::economy::{DeficitKind, Stake};
use crate::needs::{Hearth, Needs};
use crate::physics::{Blocker, Heightfield};
use crate::transform::Transform;
use crate::world::World;

/// Side of one grid cell, in metres.
pub const CELL_SIZE: f32 = 8.0;
/// Number of cells along one side of the grid.
pub const GRID_SIDE: usize = 64;
/// Total number of cells the grid scores — `GRID_SIDE * GRID_SIDE`.
pub const GRID_CELLS: usize = GRID_SIDE * GRID_SIDE;
/// Side of the whole grid, in metres — `GRID_SIDE * CELL_SIZE`.
pub const GRID_EXTENT: f32 = CELL_SIZE * GRID_SIDE as f32;

/// One cell of a [`PlacementGrid`], addressed by column and row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridCell {
    pub col: usize,
    pub row: usize,
}

impl GridCell {
    #[inline]
    fn index(self) -> usize {
        self.row * GRID_SIDE + self.col
    }

    #[inline]
    fn from_index(index: usize) -> Self {
        Self {
            col: index % GRID_SIDE,
            row: index / GRID_SIDE,
        }
    }
}

/// A source of water, for a drying rack's proximity term — the water half of
/// "hearth for storage, water for a dryer". Like [`Hearth`], it has no
/// position of its own: it sits at the owning entity's [`Transform`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterSource;

/// A worn path between two points, wide enough that no stake should be
/// planted across it. Endpoints are absolute world XZ, not relative to a
/// [`Transform`] — a path belongs to the ground it crosses, not to one entity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WornPath {
    pub from: Vec2,
    pub to: Vec2,
    pub width: f32,
}

/// Tunable weights behind [`PlacementGrid`]'s score, in the spirit of
/// [`crate::physics::Tuning`]: placeholder numbers until playtesting says
/// otherwise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacementWeights {
    /// How much a flat cell (`normal.y` near the heightfield) is worth.
    pub flatness: f32,
    /// How much being near this stake kind's service target is worth.
    pub target: f32,
    /// Metres at which the target-proximity term has fallen to one half.
    pub target_falloff: f32,
    /// How much being at the ideal distance from settlers is worth.
    pub people: f32,
    /// The distance from settlers a cell scores best at — near enough to be
    /// useful, far enough not to crowd anyone already there.
    pub ideal_people_distance: f32,
    /// Distance either side of [`PlacementWeights::ideal_people_distance`]
    /// over which the people term falls from its peak to zero.
    pub people_falloff: f32,
    /// Radius around an active stake's own cell that counts as claimed.
    pub claim_radius: f32,
}

impl Default for PlacementWeights {
    fn default() -> Self {
        Self {
            flatness: 1.0,
            target: 1.0,
            target_falloff: 40.0,
            people: 0.6,
            ideal_people_distance: 20.0,
            people_falloff: 20.0,
            claim_radius: 4.0,
        }
    }
}

fn deficit_slot(kind: DeficitKind) -> usize {
    match kind {
        DeficitKind::Shelter => 0,
        DeficitKind::Warmth => 1,
        DeficitKind::Storage => 2,
        DeficitKind::Drying => 3,
    }
}

const DEFICIT_KINDS: [DeficitKind; 4] = [
    DeficitKind::Shelter,
    DeficitKind::Warmth,
    DeficitKind::Storage,
    DeficitKind::Drying,
];

#[inline]
fn horizontal(position: Vec3) -> Vec2 {
    Vec2::new(position.x, position.z)
}

fn nearest_distance(point: Vec2, others: &[Vec2]) -> Option<f32> {
    others
        .iter()
        .map(|&o| (o - point).length())
        .fold(None, |nearest, d| match nearest {
            Some(n) if n <= d => Some(n),
            _ => Some(d),
        })
}

/// A bump that peaks at `ideal` and falls to zero `falloff` metres either
/// side of it — never negative, so a distance far outside the bump just
/// contributes nothing rather than actively penalising a cell.
fn bump(distance: f32, ideal: f32, falloff: f32) -> f32 {
    let t = (distance - ideal) / falloff;
    (1.0 - t * t).max(0.0)
}

fn blocker_radius(blocker: &Blocker) -> f32 {
    match *blocker {
        Blocker::Cylinder { radius, .. } => radius,
        Blocker::Box { half, .. } => half.x.max(half.y),
    }
}

/// A 512x512 m grid of 8 m cells (4096 cells) scoring where a stake for a
/// given [`DeficitKind`] belongs — `12-minds.md` §2-3: the settlement mind's
/// one action is planting a stake, and this is the "where".
///
/// Each cell is scored on ground flatness (reusing the same
/// [`Heightfield`] the ground is walked and drawn from), distance to whatever
/// the stake serves (a [`Hearth`] for storage, a [`WaterSource`] for drying —
/// not every kind has one, and a kind with none simply carries no such term,
/// no pathfinding involved either way), distance to settlers (too close is
/// crowding, too far is nobody's problem to solve), and excludes ground
/// already built on ([`Blocker`]), claimed by another active [`Stake`], or
/// crossed by a [`WornPath`]. The whole grid is read every time — there is no
/// fog of war for the planner.
///
/// Scoring all 4096 cells means reading every settler, hearth, blocker and
/// stake in the world, which is too much to redo every tick for a grid
/// nothing queries that often. [`PlacementGrid::recompute`] only does that
/// work when [`PlacementGrid::mark_dirty`] has been called since the last
/// time it ran — a stake built, burned, dismantled, or a path cleared; this
/// type does not call it itself and knows nothing about ticks.
#[derive(Debug, Clone)]
pub struct PlacementGrid {
    origin: Vec2,
    weights: PlacementWeights,
    scores: [Vec<f32>; 4],
    dirty: bool,
    recompute_count: u32,
}

impl PlacementGrid {
    /// A grid covering `GRID_EXTENT` x `GRID_EXTENT` metres with `origin` as
    /// its `(col: 0, row: 0)` corner. Every cell reads as excluded
    /// ([`PlacementGrid::best_cell_for`] returns `None`) until the first
    /// [`PlacementGrid::recompute`].
    pub fn new(origin: Vec2, weights: PlacementWeights) -> Self {
        Self {
            origin,
            weights,
            scores: std::array::from_fn(|_| vec![f32::NEG_INFINITY; GRID_CELLS]),
            dirty: true,
            recompute_count: 0,
        }
    }

    /// World XZ of the centre of `cell`.
    pub fn cell_center(&self, cell: GridCell) -> Vec2 {
        self.origin
            + Vec2::new(
                (cell.col as f32 + 0.5) * CELL_SIZE,
                (cell.row as f32 + 0.5) * CELL_SIZE,
            )
    }

    fn cell_bounds(&self, cell: GridCell) -> (Vec2, Vec2) {
        let min = self.origin + Vec2::new(cell.col as f32, cell.row as f32) * CELL_SIZE;
        (min, min + Vec2::splat(CELL_SIZE))
    }

    fn circle_intersects_cell(&self, center: Vec2, radius: f32, cell: GridCell) -> bool {
        let (min, max) = self.cell_bounds(cell);
        let closest = Vec2::new(center.x.clamp(min.x, max.x), center.y.clamp(min.y, max.y));
        (center - closest).length() <= radius
    }

    fn path_intersects_cell(&self, path: &WornPath, cell: GridCell) -> bool {
        let span = path.to - path.from;
        let steps = (span.length() / (CELL_SIZE * 0.5)).ceil().max(1.0) as usize;
        let half_width = path.width * 0.5;
        (0..=steps).any(|i| {
            let t = i as f32 / steps as f32;
            let point = path.from + span * t;
            self.circle_intersects_cell(point, half_width, cell)
        })
    }

    /// Tell the grid something has changed on the ground it scores — a stake
    /// built, burned, dismantled, or a path cleared. The next
    /// [`PlacementGrid::recompute`] call does the work; this call alone does
    /// not.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Whether [`PlacementGrid::recompute`] would actually do anything if
    /// called right now.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// How many times [`PlacementGrid::recompute`] has actually scored the
    /// grid, as opposed to no-oping because nothing was dirty.
    pub fn recompute_count(&self) -> u32 {
        self.recompute_count
    }

    /// Score every cell from `world` and `ground`, unless nothing has marked
    /// the grid dirty since the last time this ran, in which case this is a
    /// no-op (see [`PlacementGrid::recompute_count`]).
    pub fn recompute(&mut self, world: &World, ground: &Heightfield) {
        if !self.dirty {
            return;
        }
        self.recompute_count += 1;

        let settlers: Vec<Vec2> = world
            .iter::<Needs>()
            .filter_map(|(e, _)| world.get::<Transform>(e))
            .map(|t| horizontal(t.position))
            .collect();
        let hearths: Vec<Vec2> = world
            .iter::<Hearth>()
            .filter_map(|(e, _)| world.get::<Transform>(e))
            .map(|t| horizontal(t.position))
            .collect();
        let water_sources: Vec<Vec2> = world
            .iter::<WaterSource>()
            .filter_map(|(e, _)| world.get::<Transform>(e))
            .map(|t| horizontal(t.position))
            .collect();

        let mut occupied_circles: Vec<(Vec2, f32)> = Vec::new();
        for (entity, blocker) in world.iter::<Blocker>() {
            if let Some(transform) = world.get::<Transform>(entity) {
                occupied_circles.push((horizontal(transform.position), blocker_radius(blocker)));
            }
        }
        for (entity, stake) in world.iter::<Stake>() {
            if stake.is_abandoned() {
                continue;
            }
            if let Some(transform) = world.get::<Transform>(entity) {
                occupied_circles.push((horizontal(transform.position), self.weights.claim_radius));
            }
        }
        let worn_paths: Vec<&WornPath> = world.iter::<WornPath>().map(|(_, p)| p).collect();

        let mut flatness = vec![0.0f32; GRID_CELLS];
        let mut people_term = vec![0.0f32; GRID_CELLS];
        let mut occupied = vec![false; GRID_CELLS];

        for row in 0..GRID_SIDE {
            for col in 0..GRID_SIDE {
                let cell = GridCell { col, row };
                let idx = cell.index();
                let center = self.cell_center(cell);
                flatness[idx] = ground.normal_at(center.x, center.y).y.clamp(0.0, 1.0);
                people_term[idx] = match nearest_distance(center, &settlers) {
                    Some(d) => bump(
                        d,
                        self.weights.ideal_people_distance,
                        self.weights.people_falloff,
                    ),
                    None => 0.0,
                };
                occupied[idx] = occupied_circles
                    .iter()
                    .any(|&(c, r)| self.circle_intersects_cell(c, r, cell))
                    || worn_paths
                        .iter()
                        .any(|p| self.path_intersects_cell(p, cell));
            }
        }

        for kind in DEFICIT_KINDS {
            let slot = deficit_slot(kind);
            let targets: &[Vec2] = match kind {
                DeficitKind::Storage => &hearths,
                DeficitKind::Drying => &water_sources,
                DeficitKind::Shelter | DeficitKind::Warmth => &[],
            };
            for row in 0..GRID_SIDE {
                for col in 0..GRID_SIDE {
                    let cell = GridCell { col, row };
                    let idx = cell.index();
                    if occupied[idx] {
                        self.scores[slot][idx] = f32::NEG_INFINITY;
                        continue;
                    }
                    let center = self.cell_center(cell);
                    let target_term = match nearest_distance(center, targets) {
                        Some(d) => 1.0 / (1.0 + d / self.weights.target_falloff),
                        None => 0.0,
                    };
                    self.scores[slot][idx] = self.weights.flatness * flatness[idx]
                        + self.weights.target * target_term
                        + self.weights.people * people_term[idx];
                }
            }
        }

        self.dirty = false;
    }

    /// This cell's cached score for `kind`, as of the last
    /// [`PlacementGrid::recompute`] — `f32::NEG_INFINITY` if it is excluded
    /// (built on, claimed, path-crossed) or the grid has never been
    /// recomputed.
    pub fn score(&self, kind: DeficitKind, cell: GridCell) -> f32 {
        self.scores[deficit_slot(kind)][cell.index()]
    }

    /// The best cell for a stake of `kind`, from the scores as of the last
    /// [`PlacementGrid::recompute`]. `None` if every cell is excluded — the
    /// caller has no ground left to plant on.
    pub fn best_cell_for(&self, kind: DeficitKind) -> Option<GridCell> {
        let scores = &self.scores[deficit_slot(kind)];
        let mut best: Option<(usize, f32)> = None;
        for (idx, &score) in scores.iter().enumerate() {
            if !score.is_finite() {
                continue;
            }
            let is_better = match best {
                Some((_, best_score)) => score > best_score,
                None => true,
            };
            if is_better {
                best = Some((idx, score));
            }
        }
        best.map(|(idx, _)| GridCell::from_index(idx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::Tag;

    /// One heightfield node per placement-grid cell corner, so a node column
    /// index lines up with the world x the same column of grid cells covers.
    const GROUND_SIDE: usize = GRID_SIDE + 1;

    fn flat_ground() -> Heightfield {
        Heightfield::new(
            Vec2::ZERO,
            CELL_SIZE,
            GROUND_SIDE,
            GROUND_SIDE,
            vec![0.0; GROUND_SIDE * GROUND_SIDE],
        )
    }

    /// A field flat everywhere except a steep cliff from world x = 400 m on,
    /// well away from the origin corner where the other actors in these
    /// tests live.
    fn ground_with_a_cliff() -> Heightfield {
        let mut heights = vec![0.0f32; GROUND_SIDE * GROUND_SIDE];
        for row in 0..GROUND_SIDE {
            for col in 50..GROUND_SIDE {
                heights[row * GROUND_SIDE + col] = (col - 49) as f32 * 20.0;
            }
        }
        Heightfield::new(Vec2::ZERO, CELL_SIZE, GROUND_SIDE, GROUND_SIDE, heights)
    }

    fn spawn_hearth(world: &mut World, position: Vec3) {
        let hearth = world.spawn();
        world.insert(hearth, Transform::from_position(position));
        world.insert(hearth, Hearth::new(3.0));
    }

    fn spawn_settler(world: &mut World, position: Vec3) {
        let settler = world.spawn();
        world.insert(settler, Transform::from_position(position));
        world.insert(settler, Needs::default());
    }

    #[test]
    fn a_flat_empty_cell_near_a_hearth_outscores_a_steep_or_crowded_one() {
        let mut world = World::new();
        // The hearth Storage cares about, and a crowd far away from it.
        spawn_hearth(&mut world, Vec3::new(100.0, 0.0, 100.0));
        for _ in 0..8 {
            spawn_settler(&mut world, Vec3::new(300.0, 0.0, 300.0));
        }

        let mut grid = PlacementGrid::new(Vec2::ZERO, PlacementWeights::default());
        grid.recompute(&world, &ground_with_a_cliff());

        // Flat, empty, right by the hearth.
        let near_hearth = GridCell { col: 12, row: 12 }; // centre (100, 100)
                                                         // Steep: on the cliff, far from both the hearth and the crowd.
        let steep = GridCell { col: 55, row: 12 }; // centre (444, 100)
                                                   // Crowded: flat, but standing on top of the settlers.
        let crowded = GridCell { col: 37, row: 37 }; // centre (300, 300)

        let near_hearth_score = grid.score(DeficitKind::Storage, near_hearth);
        assert!(
            near_hearth_score > grid.score(DeficitKind::Storage, steep),
            "flat ground near the hearth should outscore a cliff far from it"
        );
        assert!(
            near_hearth_score > grid.score(DeficitKind::Storage, crowded),
            "flat ground near the hearth should outscore standing on top of a crowd"
        );
    }

    #[test]
    fn distance_to_settlers_peaks_in_the_middle_not_at_either_extreme() {
        let mut world = World::new();
        spawn_settler(&mut world, Vec3::new(0.0, 0.0, 0.0));

        let mut grid = PlacementGrid::new(Vec2::ZERO, PlacementWeights::default());
        grid.recompute(&world, &flat_ground());

        let right_on_top = GridCell { col: 0, row: 0 };
        let ideal_distance = GridCell { col: 2, row: 0 }; // centre (20, 4), ~20m out
        let far_away = GridCell { col: 63, row: 63 };

        let ideal_score = grid.score(DeficitKind::Shelter, ideal_distance);
        assert!(ideal_score > grid.score(DeficitKind::Shelter, right_on_top));
        assert!(ideal_score > grid.score(DeficitKind::Shelter, far_away));
    }

    #[test]
    fn occupied_claimed_and_path_crossed_cells_are_excluded() {
        let mut world = World::new();
        spawn_hearth(&mut world, Vec3::new(0.0, 0.0, 0.0));

        // Built on.
        let built = world.spawn();
        world.insert(built, Transform::from_position(Vec3::new(20.0, 0.0, 20.0)));
        world.insert(
            built,
            Blocker::Cylinder {
                radius: 1.0,
                top: 2.0,
            },
        );

        // Claimed by an active stake.
        let claimed = world.spawn();
        world.insert(
            claimed,
            Transform::from_position(Vec3::new(40.0, 0.0, 40.0)),
        );
        world.insert(
            claimed,
            Stake::new(DeficitKind::Warmth, Tag::Hard, 10, 100, 0),
        );

        // Crossed by a worn path.
        let path_holder = world.spawn();
        world.insert(
            path_holder,
            WornPath {
                from: Vec2::new(60.0, 0.0),
                to: Vec2::new(60.0, 80.0),
                width: 2.0,
            },
        );

        let mut grid = PlacementGrid::new(Vec2::ZERO, PlacementWeights::default());
        grid.recompute(&world, &flat_ground());

        let built_cell = GridCell { col: 2, row: 2 };
        let claimed_cell = GridCell { col: 5, row: 5 };
        let path_cell = GridCell { col: 7, row: 5 };

        for kind in DEFICIT_KINDS {
            assert!(
                !grid.score(kind, built_cell).is_finite(),
                "built-on cell must be excluded for {kind:?}"
            );
            assert!(
                !grid.score(kind, claimed_cell).is_finite(),
                "claimed cell must be excluded for {kind:?}"
            );
            assert!(
                !grid.score(kind, path_cell).is_finite(),
                "path-crossed cell must be excluded for {kind:?}"
            );
            let best = grid
                .best_cell_for(kind)
                .expect("plenty of open ground left");
            assert_ne!(best, built_cell);
            assert_ne!(best, claimed_cell);
            assert_ne!(best, path_cell);
        }
    }

    #[test]
    fn an_abandoned_stake_does_not_claim_its_cell() {
        let mut world = World::new();
        let entity = world.spawn();
        world.insert(entity, Transform::from_position(Vec3::new(40.0, 0.0, 40.0)));
        let mut stake = Stake::new(DeficitKind::Warmth, Tag::Hard, 10, 100, 0);
        stake.advance(2 * 100); // two idle sim-days: abandoned.
        assert!(stake.is_abandoned());
        world.insert(entity, stake);

        let mut grid = PlacementGrid::new(Vec2::ZERO, PlacementWeights::default());
        grid.recompute(&world, &flat_ground());

        assert!(grid
            .score(DeficitKind::Warmth, GridCell { col: 5, row: 5 })
            .is_finite());
    }

    #[test]
    fn recompute_only_scores_the_grid_when_told_something_changed() {
        let world = World::new();
        let mut grid = PlacementGrid::new(Vec2::ZERO, PlacementWeights::default());
        assert!(grid.is_dirty(), "a fresh grid has never been scored");

        grid.recompute(&world, &flat_ground());
        assert_eq!(grid.recompute_count(), 1);
        assert!(!grid.is_dirty());

        // Repeated calls with nothing marking the grid dirty do not rescore.
        grid.recompute(&world, &flat_ground());
        grid.recompute(&world, &flat_ground());
        assert_eq!(grid.recompute_count(), 1);

        grid.mark_dirty();
        assert!(grid.is_dirty());
        grid.recompute(&world, &flat_ground());
        assert_eq!(grid.recompute_count(), 2);
    }

    #[test]
    fn a_grid_that_has_never_recomputed_offers_nothing() {
        let grid = PlacementGrid::new(Vec2::ZERO, PlacementWeights::default());
        assert_eq!(grid.best_cell_for(DeficitKind::Shelter), None);
    }

    #[test]
    fn kinds_with_no_service_target_still_score_on_flatness_and_people() {
        let mut world = World::new();
        spawn_settler(&mut world, Vec3::new(100.0, 0.0, 100.0));

        let mut grid = PlacementGrid::new(Vec2::ZERO, PlacementWeights::default());
        grid.recompute(&world, &flat_ground());

        let ideal_band = GridCell { col: 15, row: 12 };
        let far_away = GridCell { col: 63, row: 63 };
        assert!(
            grid.score(DeficitKind::Shelter, ideal_band)
                > grid.score(DeficitKind::Shelter, far_away),
            "even with no service target, distance to settlers should still matter"
        );
    }

    #[test]
    fn a_fully_occupied_grid_has_no_best_cell() {
        let mut world = World::new();
        // One giant blocker over the entire grid.
        let entity = world.spawn();
        world.insert(
            entity,
            Transform::from_position(Vec3::new(GRID_EXTENT / 2.0, 0.0, GRID_EXTENT / 2.0)),
        );
        world.insert(
            entity,
            Blocker::Cylinder {
                radius: GRID_EXTENT,
                top: 1.0,
            },
        );

        let mut grid = PlacementGrid::new(Vec2::ZERO, PlacementWeights::default());
        grid.recompute(&world, &flat_ground());

        assert_eq!(grid.best_cell_for(DeficitKind::Shelter), None);
    }
}
