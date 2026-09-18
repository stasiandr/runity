//! Rigid body physics: shapes, bodies, collisions and constraints.

#![forbid(unsafe_code)]

pub mod body;
pub mod broadphase;
pub mod collide;
pub mod raycast;
pub mod shape;
pub mod solver;

pub use body::{BodyType, Material, RigidBody};
pub use broadphase::{BroadPhase, Proxy};
pub use collide::{collide, ContactPoint, Manifold};
pub use raycast::{ray_shape, Ray, RayHit};
pub use shape::{Aabb, Isometry, MassProperties, Shape};
pub use solver::{ContactSolver, ImpulseCache, PairManifold, SolverSettings};
