//! Deterministic simulation primitives.
//!
//! Nothing in this module (or its submodules) calls a transcendental function
//! (`sin`, `cos`, `tan`, `atan2`, `powf`, `to_radians`, ...). Addition,
//! subtraction, multiplication, division and `sqrt` round identically on
//! every IEEE-754 platform; transcendental functions do not carry the same
//! guarantee, and a bit of drift after a million physics ticks is not
//! acceptable. See `docs/ARCHITECTURE.md` for why golden-image tests get a
//! tolerance and this module does not.
//!
//! That rule shapes the interfaces, not just the bodies of the functions.
//! Direction arrives as a vector ([`Command::Move`]), never as an angle;
//! a slope limit is a cosine ([`Tuning::max_slope_cos`]), compared with a
//! normal's `y`; a falling trunk carries its lean as a unit vector rather than
//! a radian count. Wherever a game really does need trigonometry — a mouse
//! delta becoming a heading — it belongs in the calling code, which is free to
//! call `sin` as often as it likes.
//!
//! There is no rigid-body solver and there is not going to be one. The valley
//! needs cylinders that do not walk through each other, ground that holds them
//! up, and a tree that falls over. That is cylinders, planes and cosines:
//!
//! * [`Heightfield`] — the ground, shared with the mesh that draws it.
//! * [`Body`] — an upright cylinder; [`Body::position`] is the only truth
//!   about where it is, and [`Body::render_position`] the only way to draw it.
//! * [`Blocker`] — a component on an entity: the shapes that stand still.
//! * [`PhysicsWorld`] — the ground, the [`Tuning`], and the one spatial index.
//! * [`PendingInput`] — the latch that stops a jump falling between a frame
//!   and a tick.
//! * [`FallingTree`] — a pendulum at a stump, which becomes a log.
//!
//! ```
//! use runity_core::physics::{Body, PhysicsWorld, Tuning, Heightfield, PendingInput, Command};
//! use runity_core::World;
//! use runity_math::{Vec2, Vec3};
//!
//! let ground = Heightfield::new(Vec2::splat(-20.0), 4.0, 11, 11, vec![0.0; 121]);
//! let mut physics = PhysicsWorld::new(ground, Tuning::default(), 1.0 / 20.0);
//!
//! let mut world = World::new();
//! let walker = world.spawn();
//! world.insert(walker, Body::new(Vec3::ZERO));
//! world.insert(walker, PendingInput::new());
//!
//! // A frame's worth of intent, then a tick.
//! world.get_mut::<PendingInput>(walker).unwrap().push(Command::Move {
//!     direction: Vec3::new(0.0, 0.0, 1.0),
//!     speed_scale: 1.0,
//! });
//! physics.step(&mut world);
//!
//! let body = world.get::<Body>(walker).unwrap();
//! assert!(body.position.z > 0.0 && body.grounded);
//! ```

pub mod blocker;
pub mod body;
pub mod command;
pub mod heightfield;
pub mod space;
pub mod step;
pub mod tree;

pub use blocker::{Blocker, StaticGrid, StaticShape};
pub use body::{Body, BodyKind};
pub use command::{Command, PendingInput, TickInput};
pub use heightfield::Heightfield;
pub use space::{PhysicsWorld, Support, Tuning, SLOPE_40_DEGREES_COS};
pub use step::{step, LOG_TOP};
pub use tree::{FallingTree, QUARTER_TURN};
