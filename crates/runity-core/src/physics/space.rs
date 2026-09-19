//! The world physics runs in: the ground, the numbers, and the static index.

use super::blocker::{StaticGrid, StaticShape};
use super::heightfield::Heightfield;
use crate::world::World;
use runity_math::{Vec2, Vec3};

/// Cosine of forty degrees — the steepest ground a body can walk up.
///
/// A cosine and not an angle, because comparing it costs a dot product and
/// comparing an angle costs an `acos`. `Tuning` is built once when the world
/// is; nothing downstream ever needs the angle back.
pub const SLOPE_40_DEGREES_COS: f32 = 0.766_044_4;

/// Every number the simulation has. Built once, read every tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tuning {
    /// Metres per second squared, downwards.
    pub gravity: f32,
    /// Fastest a body may travel under its own power.
    pub max_speed: f32,
    /// Seconds to go from a standstill to [`Tuning::max_speed`].
    pub acceleration_time: f32,
    /// Cosine of the steepest walkable slope — see [`SLOPE_40_DEGREES_COS`].
    pub max_slope_cos: f32,
    /// How high a body steps up without leaving the ground. The same number
    /// decides what blocks it: a blocker whose top is more than this above the
    /// foot is a wall, and anything lower is a step.
    pub step_height: f32,
    /// Upward speed a jump starts with. 3.43 m/s peaks at 0.6 m and is back
    /// on the ground 0.7 s later — fourteen ticks at 20 Hz.
    pub jump_speed: f32,
    /// Smallest radius any body in this world has. Guards the anti-tunnelling
    /// invariant in [`PhysicsWorld::new`].
    pub min_body_radius: f32,
    /// Side of a cell in the static grid.
    pub static_cell: f32,
    /// Constant angular acceleration on a falling trunk. `2 * (pi/2) / 1.5^2`
    /// — a quarter turn in the second and a half a tree takes to come down.
    pub tree_torque: f32,
    /// How hard a falling trunk shoves a body out of its way.
    pub tree_push_speed: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            gravity: 9.81,
            max_speed: 3.4,
            acceleration_time: 0.12,
            max_slope_cos: SLOPE_40_DEGREES_COS,
            step_height: 0.4,
            jump_speed: 3.43,
            min_body_radius: 0.3,
            static_cell: 4.0,
            tree_torque: 1.396_263_4,
            tree_push_speed: 3.0,
        }
    }
}

impl Tuning {
    /// Metres per second squared a body may change its horizontal velocity by.
    #[inline]
    pub fn acceleration(&self) -> f32 {
        self.max_speed / self.acceleration_time
    }
}

/// What a body's feet are resting on at a given spot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Support {
    /// World height of the surface.
    pub height: f32,
    /// Its upward normal. `normal.y` is the cosine of its slope, which is what
    /// [`Tuning::max_slope_cos`] is compared against.
    pub normal: Vec3,
    /// True when the surface is the top of a [`super::Blocker`] rather than
    /// the heightfield — a body standing on a stump is held up by the stump.
    pub on_blocker: bool,
}

impl Support {
    /// Whether this surface is too steep to stand on.
    #[inline]
    pub fn is_steep(&self, tuning: &Tuning) -> bool {
        self.normal.y < tuning.max_slope_cos
    }

    /// Unit horizontal direction straight down the slope, or zero on the flat.
    ///
    /// For a surface `y = f(x, z)` the normal is proportional to
    /// `(-f_x, 1, -f_z)`, so the downhill gradient is `(n.x, n.z)` — no
    /// trigonometry anywhere in sight.
    #[inline]
    pub fn downhill(&self) -> Vec2 {
        Vec2::new(self.normal.x, self.normal.z).normalized()
    }
}

/// The ground, the numbers, and the index of everything that stands still.
///
/// Physics is handed a fixed delta once, at construction, and never sees a
/// frame delta at all. That is not tidiness: it is the reason two runs of the
/// same recorded input at 60 and at 120 frames per second produce bit-identical
/// positions on every tick.
pub struct PhysicsWorld {
    /// The ground bodies walk on, shared with whatever draws it.
    pub ground: Heightfield,
    pub tuning: Tuning,
    fixed_delta: f32,
    grid: StaticGrid,
    statics_dirty: bool,
}

impl PhysicsWorld {
    /// Build the world.
    ///
    /// # Panics (debug builds)
    ///
    /// A body that moves further in one tick than its own radius can start a
    /// tick on one side of a wall and end it on the other, with no tick in
    /// between where they overlap — the classic tunnel. The check is a
    /// `debug_assert` and not a runtime one because it is a property of the
    /// numbers, not of the run: if it holds once at startup it holds forever.
    /// At the valley's 3.4 m/s and 20 Hz the margin is 1.76.
    pub fn new(ground: Heightfield, tuning: Tuning, fixed_delta: f32) -> Self {
        assert!(fixed_delta > 0.0, "a physics tick must take some time");
        debug_assert!(
            tuning.max_speed * fixed_delta < tuning.min_body_radius,
            "a body moving {} m/s covers {} m per {fixed_delta} s tick, which is \
             more than its {} m radius: it can tunnel through a wall",
            tuning.max_speed,
            tuning.max_speed * fixed_delta,
            tuning.min_body_radius
        );
        let cell = tuning.static_cell;
        Self {
            ground,
            tuning,
            fixed_delta,
            grid: StaticGrid::new(cell),
            statics_dirty: true,
        }
    }

    /// The tick length physics was built with.
    #[inline]
    pub fn fixed_delta(&self) -> f32 {
        self.fixed_delta
    }

    /// The static index, for a caller that wants to look at it.
    #[inline]
    pub fn statics(&self) -> &StaticGrid {
        &self.grid
    }

    /// Tell physics the statics have changed — a tree felled, a wall built.
    /// The grid is rebuilt on the next tick, not now.
    #[inline]
    pub fn mark_statics_dirty(&mut self) {
        self.statics_dirty = true;
    }

    #[inline]
    pub fn statics_are_dirty(&self) -> bool {
        self.statics_dirty
    }

    /// Rebuild the grid if anything has marked it dirty. Called at the top of
    /// every tick; costs a branch when nothing has changed.
    pub fn sync_statics(&mut self, world: &World) {
        if self.statics_dirty {
            self.grid.rebuild(world);
            self.statics_dirty = false;
        }
    }

    /// What holds a body up at `(x, z)` when its feet are at `foot`.
    ///
    /// The one rule that decides both stepping and jumping lives here: a
    /// blocker whose top is more than [`Tuning::step_height`] above `foot` is
    /// a wall and takes no part in this, and anything lower is something to
    /// stand on. So while a body is over a low obstacle its support is that
    /// obstacle's top, not the ground underneath it.
    pub fn support_at(&self, x: f32, z: f32, foot: f32, radius: f32) -> Support {
        let mut nearby = Vec::new();
        self.support_with(x, z, foot, radius, &mut nearby)
    }

    /// As [`PhysicsWorld::support_at`], reusing a caller's query buffer so a
    /// tick with a hundred bodies in it allocates nothing.
    pub(super) fn support_with(
        &self,
        x: f32,
        z: f32,
        foot: f32,
        radius: f32,
        nearby: &mut Vec<u32>,
    ) -> Support {
        let mut support = Support {
            height: self.ground.height_at(x, z),
            normal: self.ground.normal_at(x, z),
            on_blocker: false,
        };
        let ceiling = foot + self.tuning.step_height;
        let point = Vec2::new(x, z);
        self.grid.query(x, z, nearby);
        for &index in nearby.iter() {
            let shape = self.grid.shape(index);
            let top = shape.top();
            if top > ceiling || top <= support.height {
                continue;
            }
            if shape.covers(point, radius) {
                support = Support {
                    height: top,
                    normal: Vec3::Y,
                    on_blocker: true,
                };
            }
        }
        support
    }

    /// Every static shape in the nine cells around `(x, z)`, by index.
    #[inline]
    pub(super) fn nearby_statics(&self, x: f32, z: f32, out: &mut Vec<u32>) {
        self.grid.query(x, z, out);
    }

    #[inline]
    pub(super) fn shape(&self, index: u32) -> &StaticShape {
        self.grid.shape(index)
    }

    /// Run one fixed tick. See [`super::step`].
    pub fn step(&mut self, world: &mut World) {
        super::step::step(self, world);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::Blocker;
    use crate::transform::Transform;

    fn flat_world() -> PhysicsWorld {
        let ground = Heightfield::new(Vec2::new(-20.0, -20.0), 4.0, 11, 11, vec![0.0; 121]);
        PhysicsWorld::new(ground, Tuning::default(), 1.0 / 20.0)
    }

    #[test]
    fn the_default_tuning_satisfies_the_anti_tunnelling_invariant() {
        let tuning = Tuning::default();
        let travel = tuning.max_speed / 20.0;
        assert!(travel < tuning.min_body_radius, "{travel}");
        assert!(
            (tuning.min_body_radius / travel - 1.76).abs() < 0.01,
            "the margin the design claims is 1.76, got {}",
            tuning.min_body_radius / travel
        );
    }

    #[test]
    #[should_panic(expected = "tunnel")]
    fn a_tick_long_enough_to_tunnel_is_caught_when_the_world_is_built() {
        let ground = Heightfield::new(Vec2::ZERO, 1.0, 2, 2, vec![0.0; 4]);
        // A tenth of a second at 3.4 m/s is 0.34 m — further than a body is wide.
        let _ = PhysicsWorld::new(ground, Tuning::default(), 0.1);
    }

    #[test]
    fn support_is_the_ground_where_nothing_stands() {
        let physics = flat_world();
        let support = physics.support_at(1.0, 1.0, 0.0, 0.3);
        assert_eq!(support.height, 0.0);
        assert_eq!(support.normal, Vec3::Y);
        assert!(!support.on_blocker);
        assert!(!support.is_steep(&physics.tuning));
    }

    #[test]
    fn a_low_blocker_becomes_the_floor_and_a_tall_one_does_not() {
        let mut world = World::new();
        let step = world.spawn();
        world.insert(step, Transform::from_position(Vec3::new(2.0, 0.0, 0.0)));
        world.insert(
            step,
            Blocker::Cylinder {
                radius: 1.0,
                top: 0.3,
            },
        );
        let wall = world.spawn();
        world.insert(wall, Transform::from_position(Vec3::new(6.0, 0.0, 0.0)));
        world.insert(
            wall,
            Blocker::Cylinder {
                radius: 1.0,
                top: 3.0,
            },
        );

        let mut physics = flat_world();
        physics.sync_statics(&world);

        let on_step = physics.support_at(2.0, 0.0, 0.0, 0.3);
        assert_eq!(on_step.height, 0.3);
        assert!(on_step.on_blocker);

        let at_wall = physics.support_at(6.0, 0.0, 0.0, 0.3);
        assert_eq!(at_wall.height, 0.0, "a wall is not something to stand on");
        assert!(!at_wall.on_blocker);

        // From the top of the wall, though, the wall is exactly that.
        let atop_wall = physics.support_at(6.0, 0.0, 3.0, 0.3);
        assert_eq!(atop_wall.height, 3.0);
        assert!(atop_wall.on_blocker);
    }

    #[test]
    fn the_grid_is_rebuilt_only_when_something_marks_it_dirty() {
        let mut world = World::new();
        let mut physics = flat_world();
        assert!(physics.statics_are_dirty(), "nothing has been read yet");
        physics.sync_statics(&world);
        assert!(!physics.statics_are_dirty());
        assert_eq!(physics.statics().len(), 0);

        let tree = world.spawn();
        world.insert(tree, Transform::from_position(Vec3::ZERO));
        world.insert(
            tree,
            Blocker::Cylinder {
                radius: 0.4,
                top: 6.0,
            },
        );
        physics.sync_statics(&world);
        assert_eq!(physics.statics().len(), 0, "nobody said anything changed");

        physics.mark_statics_dirty();
        physics.sync_statics(&world);
        assert_eq!(physics.statics().len(), 1);
    }

    #[test]
    fn downhill_points_down_the_slope_and_is_zero_on_the_flat() {
        let flat = Support {
            height: 0.0,
            normal: Vec3::Y,
            on_blocker: false,
        };
        assert_eq!(flat.downhill(), Vec2::ZERO);

        // Ground rising along +X: normal leans towards -X, downhill is -X.
        let slope = Support {
            height: 0.0,
            normal: Vec3::new(-0.8, 0.6, 0.0),
            on_blocker: false,
        };
        assert!((slope.downhill() - Vec2::new(-1.0, 0.0)).length() < 1e-5);
        assert!(
            slope.is_steep(&Tuning::default()),
            "a cosine of 0.6 is a 53 degree slope"
        );

        let walkable = Support {
            normal: Vec3::new(-0.6, 0.8, 0.0),
            ..slope
        };
        assert!(
            !walkable.is_steep(&Tuning::default()),
            "a cosine of 0.8 is 37 degrees, inside the limit"
        );
    }
}
