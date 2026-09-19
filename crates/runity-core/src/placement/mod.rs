//! Where a stake belongs: a chunk-wide grid the settlement mind's stake can
//! be scored against, per `12-minds.md` §2-3.
//!
//! [`PlacementGrid`] is the whole of it — a 512x512 m grid of 8 m cells
//! (4096 cells) that [`PlacementGrid::best_cell_for`] picks the best of for a
//! given [`crate::economy::DeficitKind`]. Nothing here decides *when* to ask:
//! deciding that a stake is needed at all, and actually planting one at the
//! cell this module names, both stay a later card's job.
//!
//! ```
//! use runity_core::economy::DeficitKind;
//! use runity_core::physics::Heightfield;
//! use runity_core::placement::{PlacementGrid, PlacementWeights};
//! use runity_core::World;
//! use runity_math::Vec2;
//!
//! let world = World::new();
//! let ground = Heightfield::new(Vec2::ZERO, 8.0, 65, 65, vec![0.0; 65 * 65]);
//!
//! let mut grid = PlacementGrid::new(Vec2::ZERO, PlacementWeights::default());
//! grid.recompute(&world, &ground);
//! let cell = grid.best_cell_for(DeficitKind::Shelter);
//! assert!(cell.is_some(), "flat empty ground has somewhere to plant a hut");
//!
//! // Nothing changed since: a second call does no work.
//! grid.recompute(&world, &ground);
//! assert_eq!(grid.recompute_count(), 1);
//! ```

mod grid;

pub use grid::{
    GridCell, PlacementGrid, PlacementWeights, WaterSource, WornPath, CELL_SIZE, GRID_CELLS,
    GRID_EXTENT, GRID_SIDE,
};
