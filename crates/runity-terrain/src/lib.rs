//! Ground that is not flat.
//!
//! A half-space is enough to stand on and not enough to build a village on:
//! the moment the world has a hill, four separate systems need to agree on
//! where its surface is — the renderer draws it, the physics walks on it, the
//! pathfinder costs it, and the game places buildings on it. Four sources of
//! truth is four bugs, so there is one [`Heightmap`] and everything else asks
//! it.
//!
//! The parts:
//!
//! * [`Heightmap`] — a grid of heights, sampled bilinearly, with normals from
//!   the grid rather than from any particular mesh.
//! * [`Terrain`] — a heightmap placed in the world: world coordinates in,
//!   height, normal, slope and ray hits out.
//! * [`Chunk`] and the meshing on [`Terrain`] — triangles for the renderer,
//!   in pieces small enough to cull.

#![forbid(unsafe_code)]

mod heightmap;
mod mesh;
mod terrain;

pub use heightmap::Heightmap;
pub use mesh::{Chunk, ChunkMesh};
pub use terrain::{Terrain, TerrainHit};
