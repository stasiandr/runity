//! The physics module's components of a world, dressed from a line's
//! fields (`physics::PhysicsDress`).

use crate::scene::Body;

/// Kept from the scene so that physics can pick entities up later without the
/// scene having to be re-read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Physics(pub Body);


/// Friction, bounce and density, kept from the scene when not the default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Props(pub crate::scene::BodyProps);

/// What holds the body to another, kept from the scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Jointed(pub crate::scene::Joint);

/// How hard its joint may be pulled before it breaks, in newtons.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointBreak(pub f32);

/// Its joint broke: it is not built again until the entity's joint is set
/// anew (remove this to mend it).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct JointBroken;

/// The shape physics sees, kept from the scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape(pub crate::scene::Collider);
