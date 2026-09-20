//! The Valley: first person, a camera with a neck, twelve settlers, an axe
//! and a wall — the game laid over the world of the previous card.
//!
//! ```text
//! cargo run --release --example valley
//! RUNITY_WALKTHROUGH=1 cargo run --release --example valley  # play the recorded run
//! RUNITY_RECORD=1 cargo run --release --example valley       # the card's 60-frame video
//! RUNITY_HEADLESS=1 cargo run --release --example valley     # writes valley.png
//! ```
//!
//! | key | what it does |
//! |-----|--------------|
//! | `W` `A` `S` `D` | walk, in the direction the head is facing |
//! | mouse, arrows | turn the head — every frame, never waiting for a tick |
//! | shift | run: 3.4 m/s instead of 1.7, while there is breath for it |
//! | space | jump, latched through [`PendingInput`] so it cannot be lost |
//! | `E` | swing the axe at the nearest trunk |
//! | `B` | put a wall down where you stand, and step out of it |
//! | `T` | cycle the simulation between 60, 30, 20 and 15 Hz, live |
//! | `M` | the fixed-scale top-down debug view of the previous card |
//! | `F` | hide or show the panels in the corners |
//!
//! The world — a 512x512 m basin with a streambed and a mountain rim, a forest
//! placed by rejection sampling, one dense grove — is the previous card's and
//! is unchanged. What is new is everything that moves in it.
//!
//! Three rules shape how the pieces are split up, and they are worth stating
//! because every temptation here is to break one of them:
//!
//! * **Physics never sees an angle.** `W` and the mouse become a *vector*
//!   before they reach [`Command::Move`]; the trigonometry that turns a
//!   heading into that vector lives here, in the presentation layer, where
//!   `sin` is free (see `runity_core::physics`'s module documentation).
//! * **The head is not the body.** Everything in [`camera`] runs once per
//!   frame off [`Body::render_position`] and never writes back — the camera
//!   cannot move the player, only look like it does.
//! * **An edge is latched, a level is not.** A jump goes into
//!   [`PendingInput`] the frame the key goes down and waits there for a tick;
//!   a direction is simply overwritten, because the newest one is the right
//!   one.
//!
//! The stamina scale is a toy and is meant to be deleted: thirty lines that
//! make a run cost something and standing still give it back, standing in for
//! the real fatigue need of `docs/design/02-settlers.md` until it arrives.

use runity::prelude::*;
use runity::render::Fog;
use runity_core::physics::{
    Blocker, Body, Command, FallingTree, Heightfield, PendingInput, PhysicsWorld, StaticGrid,
    StaticShape, Tuning, LOG_TOP, SLOPE_40_DEGREES_COS,
};
use runity_core::Rng as TickRng;
use std::io;

#[path = "valley/camera.rs"]
mod camera;

// The valley's palette and daylight, from card #59. It is a registered
// example of its own (`valley-look`), so pulling it in as a module brings a
// `main` and a pile of constants this file does not use along with it.
#[path = "valley/look.rs"]
#[allow(dead_code)]
mod look;

use camera::Head;

const WIDTH: u32 = 960;
const HEIGHT: u32 = 540;

// --- terrain: a 512x512 m chunk, bowl + streambed + rim -------------------

/// Half the side of the chunk: the field spans `[-CHUNK_HALF, CHUNK_HALF]`.
const CHUNK_HALF: f32 = 256.0;
/// Radius the basin proper reaches before the rim starts climbing.
const INNER_RADIUS: f32 = 200.0;
/// How high the rim rises above the basin at the edge of the chunk.
const RIM_HEIGHT: f32 = 80.0;
/// Curvature of the basin floor: gentle enough to stay walkable everywhere.
const BASIN_K: f32 = 0.0002;
/// How far the streambed dips below the basin floor, at its centre line.
const STREAM_DEPTH: f32 = 3.0;
/// Half-width of the streambed's dip.
const STREAM_HALF_WIDTH: f32 = 15.0;

/// Height of the valley's surface at world `(x, z)`.
///
/// A bowl (`BASIN_K * r^2`) for the basin floor, a smoothstep ramp added on
/// top for the rim beyond [`INNER_RADIUS`], and a witch-of-Agnesi dip along
/// `z = 0` for the streambed — no water is drawn, the stream is simply lower
/// ground. This is example code, not the physics core, so it is free to use
/// `sin`/`cos`/`sqrt` however it likes; only `runity_core::physics` itself is
/// held to the no-transcendentals rule that keeps a tick reproducible.
fn terrain_height(x: f32, z: f32) -> f32 {
    let r = (x * x + z * z).sqrt();
    let basin = BASIN_K * r * r;
    let rim = rim_extra(r);
    let normalized = z / STREAM_HALF_WIDTH;
    let stream = -STREAM_DEPTH / (1.0 + normalized * normalized);
    basin + rim + stream
}

/// Extra height the rim adds beyond [`INNER_RADIUS`], ramping smoothly (zero
/// slope at both ends) up to [`RIM_HEIGHT`] at `r = CHUNK_HALF` and staying
/// there beyond it — the chunk's square corners reach further than the rim's
/// own radius, and a flat plateau out there is harmless.
fn rim_extra(r: f32) -> f32 {
    if r <= INNER_RADIUS {
        return 0.0;
    }
    let t = ((r - INNER_RADIUS) / (CHUNK_HALF - INNER_RADIUS)).clamp(0.0, 1.0);
    let smooth = t * t * (3.0 - 2.0 * t);
    RIM_HEIGHT * smooth
}

/// Base resolution the whole chunk is sampled at — fine enough to be a
/// believable physics ground later, and the source the coarse ring's
/// [`Heightfield::to_mesh_lod`] strides over.
const WORLD_CELL: f32 = 2.0;
/// Stride applied to [`WORLD_CELL`] to get the coarse ring's 4 m step.
const FAR_LOD_STRIDE: usize = 2;
/// Half the side of the full-resolution patch (64x64 m in total).
const PATCH_HALF: f32 = 32.0;
const PATCH_CELL: f32 = 1.0;

/// Sample the whole chunk on a `cell`-metre lattice anchored at its corner.
fn build_heightfield(cell: f32) -> Heightfield {
    let cols = (2.0 * CHUNK_HALF / cell) as usize + 1;
    let origin = Vec2::splat(-CHUNK_HALF);
    let mut heights = Vec::with_capacity(cols * cols);
    for row in 0..cols {
        let z = origin.y + row as f32 * cell;
        for col in 0..cols {
            let x = origin.x + col as f32 * cell;
            heights.push(terrain_height(x, z));
        }
    }
    Heightfield::new(origin, cell, cols, cols, heights)
}

/// The whole 512x512 m chunk at [`WORLD_CELL`] resolution — the source for
/// the coarse distant ring, and the one place `terrain_height` is sampled
/// with a fixed, world-aligned lattice.
fn build_far_heightfield() -> Heightfield {
    build_heightfield(WORLD_CELL)
}

/// The ground physics walks on: the whole chunk at [`PATCH_CELL`], the same
/// resolution the patch under the player is *drawn* at.
///
/// Sampling physics coarser than the mesh beside it is how a foot ends up
/// visibly inside a hillside: between two 2 m nodes a curved bank bulges above
/// the straight line the physics would interpolate. A megabyte of floats buys
/// the feet standing exactly on the surface a player can see.
fn build_physics_heightfield() -> Heightfield {
    build_heightfield(PATCH_CELL)
}

/// Where a patch centred on `center` is snapped to: the 4 m lattice the coarse
/// ring's stride keeps, so the patch's edge nodes land on nodes the ring
/// already drew.
fn snap_patch(center: Vec2) -> Vec2 {
    let snap = WORLD_CELL * FAR_LOD_STRIDE as f32;
    Vec2::new(
        (center.x / snap).round() * snap,
        (center.y / snap).round() * snap,
    )
}

/// A full-resolution 64x64 m patch around `center`, snapped onto the same
/// `4 m` lattice the coarse ring's stride keeps — so the patch's own edge
/// nodes land exactly on nodes the ring already drew, at heights computed by
/// the very same [`terrain_height`] call.
fn build_patch_heightfield(center: Vec2) -> Heightfield {
    let origin = snap_patch(center) - Vec2::splat(PATCH_HALF);
    let cols = (2.0 * PATCH_HALF / PATCH_CELL) as usize + 1;
    let mut heights = Vec::with_capacity(cols * cols);
    for row in 0..cols {
        let z = origin.y + row as f32 * PATCH_CELL;
        for col in 0..cols {
            let x = origin.x + col as f32 * PATCH_CELL;
            heights.push(terrain_height(x, z));
        }
    }
    Heightfield::new(origin, PATCH_CELL, cols, cols, heights)
}

// --- forest: rejection sampling, plus one dense grove ----------------------

const TREE_RADIUS: f32 = 0.35;
const MIN_TREE_SPACING: f32 = 4.0;
/// Roughly one trunk per 120 square metres — visible clean through at a
/// hundred metres.
const BASE_TREE_DENSITY: f32 = 1.0 / 120.0;
/// The sparse forest's extent: inside the basin, clear of the rim entirely.
const FOREST_RADIUS: f32 = 190.0;
/// Centre of the one deliberately dense patch, close to the stream.
const DENSE_PATCH_CENTER: Vec2 = Vec2::new(60.0, 5.0);
/// About 30 m across.
const DENSE_PATCH_RADIUS: f32 = 15.0;
const DENSE_DENSITY_MULT: f32 = 4.0;
/// Fixed so the forest — and everything anchored to it — is the same every
/// run.
const FOREST_SEED: u64 = 0xC0FF_EE15_5EED_2026;

struct Tree {
    position: Vec2,
    height: f32,
}

/// A small deterministic PRNG (SplitMix64) — the engine's own
/// [`runity_core::Rng`] is seeded from `(world, tick)` for simulation draws,
/// and the forest is not a simulation draw: it just needs a seeded,
/// repeatable stream of floats at startup.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}

/// Scatter trees over a disk by dart-throwing: propose a uniformly random
/// point, keep it only if it clears `spacing` from every tree already placed
/// (in this region or any other) and `allow` accepts it.
///
/// Rejection sampling rather than plain noise is the whole point: noise alone
/// can put two trunks a hand's width apart, which is a gap nothing can walk
/// through.
fn place_region(
    rng: &mut Rng,
    trees: &mut Vec<Tree>,
    center: Vec2,
    radius: f32,
    density: f32,
    spacing: f32,
    allow: impl Fn(Vec2) -> bool,
) {
    let area = std::f32::consts::PI * radius * radius;
    let target = (area * density).round() as usize;
    let max_attempts = (target.max(50)) * 200;
    let mut placed = 0;
    let mut attempts = 0;
    while placed < target && attempts < max_attempts {
        attempts += 1;
        // sqrt corrects for the disk's radial density, so points don't
        // bunch up near the centre.
        let r = radius * rng.unit().sqrt();
        let theta = rng.range(0.0, std::f32::consts::TAU);
        let point = center + Vec2::new(r * theta.cos(), r * theta.sin());
        if !allow(point) {
            continue;
        }
        if trees
            .iter()
            .any(|t| (t.position - point).length() < spacing)
        {
            continue;
        }
        trees.push(Tree {
            position: point,
            height: rng.range(5.0, 9.0),
        });
        placed += 1;
    }
}

/// The whole forest: the sparse basin, the dense grove by the stream, then the
/// camp's own thicket and lone tree — appended in that order, so the earlier
/// regions are exactly what the previous card placed.
fn place_forest(seed: u64) -> Vec<Tree> {
    let mut rng = Rng::new(seed);
    let mut trees = Vec::new();
    place_region(
        &mut rng,
        &mut trees,
        Vec2::ZERO,
        FOREST_RADIUS,
        BASE_TREE_DENSITY,
        MIN_TREE_SPACING,
        |p| (p - DENSE_PATCH_CENTER).length() > DENSE_PATCH_RADIUS,
    );
    place_region(
        &mut rng,
        &mut trees,
        DENSE_PATCH_CENTER,
        DENSE_PATCH_RADIUS,
        BASE_TREE_DENSITY * DENSE_DENSITY_MULT,
        MIN_TREE_SPACING,
        |_| true,
    );
    place_region(
        &mut rng,
        &mut trees,
        THICKET_CENTER,
        THICKET_RADIUS,
        THICKET_DENSITY,
        THICKET_SPACING,
        |_| true,
    );
    trees.push(Tree {
        position: LONE_TREE,
        height: 7.0,
    });
    trees
}

/// Spawn one entity per tree, each carrying the [`Transform`] and
/// [`Blocker`] the static grid reads, and the [`FallingTree`] an axe needs.
fn populate_world(world: &mut World, ground: &Heightfield, trees: &[Tree]) -> Vec<Standing> {
    let mut standing = Vec::with_capacity(trees.len());
    for tree in trees {
        let entity = world.spawn();
        let y = ground.height_at(tree.position.x, tree.position.y);
        world.insert(
            entity,
            Transform::from_position(Vec3::new(tree.position.x, y, tree.position.y)),
        );
        world.insert(
            entity,
            Blocker::Cylinder {
                radius: TREE_RADIUS,
                top: tree.height,
            },
        );
        world.insert(entity, FallingTree::new(tree.height, TREE_RADIUS));
        standing.push(Standing {
            entity,
            position: tree.position,
            base: Vec3::new(tree.position.x, y, tree.position.y),
            height: tree.height,
        });
    }
    standing
}

/// A tree as the game holds it: the entity physics knows about, and the two
/// numbers drawing it needs without a world lookup.
struct Standing {
    entity: Entity,
    position: Vec2,
    base: Vec3,
    height: f32,
}

// --- the camp -------------------------------------------------------------

/// Where the player wakes up (`docs/design/04-player.md`, "Первые десять
/// минут"): a fire someone else has been keeping, out near the eastern rim,
/// a little way down the stream.
///
/// The camp sits under the rim but the way out of it runs *along* the hill
/// rather than at it — north, across the stream. Walking straight at a
/// mountain eighty metres high fills the screen with mountain and nothing
/// else; with the rim over one shoulder the valley has a distance to it. And
/// everything the first ten minutes need — trees to walk between, a thicket
/// to pick through, a log to step over, a rock to climb and fall off the side
/// of, and ground too steep to climb at all — is inside thirty metres of the
/// fire, which is what makes a recorded walkthrough of the lot fit in a
/// quarter of a minute.
const CAMPFIRE: Vec2 = Vec2::new(186.0, -12.0);
/// The way out of the camp, as a unit vector: north and a little east, so the
/// walk runs *along* the rim while closing on it, and the rim is over one
/// shoulder instead of filling the screen. Everything below is placed on this
/// line, at the metre mark named in its comment.
const ROUTE: Vec2 = Vec2::new(0.5219, 0.8531);
/// A stand of trees by the stream, grown in tighter than the four-metre gap
/// the rest of the forest keeps: a body fits through, but has to aim.
/// Six metres up the route.
const THICKET_CENTER: Vec2 = Vec2::new(189.1, -6.9);
const THICKET_RADIUS: f32 = 4.0;
const THICKET_SPACING: f32 = 2.2;
const THICKET_DENSITY: f32 = 1.0 / 11.0;
/// One tree grown out past the edge of the forest, up where the ground starts
/// to tilt — the one the walkthrough fells.
const LONE_TREE: Vec2 = Vec2::new(198.6, -4.7);
/// A trunk that came down long before the player did, lying across the way up
/// the valley.
/// Eleven metres up the route.
const OLD_LOG: Vec2 = Vec2::new(191.7, -2.6);
/// Lying across the route, so it has to be stepped over rather than walked
/// around.
const OLD_LOG_FACING: Vec2 = Vec2::new(0.8531, -0.5219);
const OLD_LOG_LENGTH: f32 = 7.0;
/// A rock in three steps, each riser under [`Tuning::step_height`] so it is
/// walked up without a jump. The valley climbs towards the rim fast enough
/// that the far *end* of the rock is barely a step above the hillside; the
/// side of it, where the ground does not climb at all, is the metre-high
/// ledge to walk off.
/// Sixteen and a half metres up the route.
const OUTCROP: Vec2 = Vec2::new(194.6, 2.1);
/// How high each step stands over the ground the rock is *approached* from.
/// Measured from one base, not from the hillside under each step: the ground
/// climbs towards the rim, and a riser measured locally would come out over
/// the step height and turn the rock into a wall.
const OUTCROP_TOPS: [f32; 3] = [0.30, 0.64, 0.98];
const OUTCROP_HALF: Vec2 = Vec2::new(1.0, 2.0);
/// The rock is climbed along the route, and walked off the side of, which is
/// the way the rim is.
const OUTCROP_FACING: Vec2 = ROUTE;

/// The heading that walks up [`ROUTE`] — the one place in the game an angle
/// is made out of a direction, and it is a *camera* angle.
fn route_yaw() -> f32 {
    ROUTE.x.atan2(-ROUTE.y)
}

/// A log on the ground: a lying trunk to draw, and a `Box` blocker to step
/// over. Exactly what `physics::step` leaves behind when a felled tree lands.
struct Log {
    base: Vec3,
    facing: Vec2,
    length: f32,
}

/// A wall the player has put up.
struct Wall {
    base: Vec3,
    half: Vec2,
    facing: Vec2,
    top: f32,
}

/// A step of rock: a box standing on the ground.
struct Step {
    base: Vec3,
    half: Vec2,
    facing: Vec2,
    top: f32,
}

// --- the player -----------------------------------------------------------

/// Walking and running speeds (`docs/design/04-player.md`): the player has no
/// privileged mechanics, so the run is the tuning's own `max_speed`.
const WALK_SPEED: f32 = 1.7;
const RUN_SPEED: f32 = 3.4;
/// Reach of a swing of the axe, measured between axes.
const AXE_REACH: f32 = 2.6;

/// A toy stamina scale, `1.0` fresh to `0.0` spent.
///
/// Thirty lines, and they are meant to be thrown away: the real thing is a
/// need with a curve and a body that shows it (`docs/design/02-settlers.md`,
/// and the three stages of `04-player.md`). Until then this is the smallest
/// thing that makes a sprint cost something and a jump cost a little, which
/// is what the card asks to be able to feel.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Stamina(f32);

/// Running empties a full scale in ten seconds.
const STAMINA_RUN_DRAIN: f32 = 0.10;
/// Standing refills it in ten.
const STAMINA_STAND_RECOVERY: f32 = 0.10;
/// A jump costs a little — "прыжок немного тратит".
const STAMINA_JUMP_COST: f32 = 0.06;

impl Stamina {
    fn full() -> Self {
        Self(1.0)
    }

    /// One frame of breathing: running spends, standing still gives back, and
    /// walking is neither.
    fn breathe(&mut self, dt: f32, running: bool, moving: bool) {
        let change = if running {
            -STAMINA_RUN_DRAIN
        } else if moving {
            0.0
        } else {
            STAMINA_STAND_RECOVERY
        };
        self.0 = (self.0 + change * dt).clamp(0.0, 1.0);
    }

    /// What is left of the scale, `1.0` fresh to `0.0` spent — for the bar in
    /// the corner, which is the only way the toy is visible on its own rather
    /// than through a run that has gone short.
    fn remaining(self) -> f32 {
        self.0
    }

    /// Whether there is enough breath left to leave the ground at all.
    fn can_jump(self) -> bool {
        self.0 >= STAMINA_JUMP_COST
    }

    fn spend_jump(&mut self) {
        self.0 = (self.0 - STAMINA_JUMP_COST).max(0.0);
    }

    /// The fraction of [`Tuning::max_speed`] to ask for.
    ///
    /// A walk is a walk however tired; it is the *run* that shortens, all the
    /// way down to a walk when there is nothing left.
    fn speed_scale(self, sprint: bool) -> f32 {
        let walk = WALK_SPEED / RUN_SPEED;
        if sprint {
            walk + (1.0 - walk) * self.0
        } else {
            walk
        }
    }

    /// How hard the legs push off. Height goes as the square of this, so an
    /// exhausted jump clears about half of a fresh one.
    fn jump_speed(self) -> f32 {
        Tuning::default().jump_speed * (0.7 + 0.3 * self.0)
    }
}

/// One frame's worth of what a person — or a recorded script — is asking for.
///
/// Levels (`walk`, `sprint`) hold; edges (`jump`, `chop`, `build`,
/// `tick_rate`) are true on exactly the frame they happen.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Intent {
    /// Right/left in `x`, forward/back in `y`, each in `[-1, 1]`, in the
    /// head's own frame.
    walk: Vec2,
    /// Mouse movement this frame, in pixels.
    look: Vec2,
    sprint: bool,
    jump: bool,
    chop: bool,
    build: bool,
    tick_rate: bool,
}

/// How fast the arrow keys turn the head, in degrees per second.
///
/// The mouse is the card's answer and the one the camera is tuned for, but
/// `Input::mouse_delta` is the difference of two *positions inside the window*
/// — there is no pointer lock in `runity-platform` yet — so a turn that runs
/// the cursor into the edge of the window simply stops. The arrows are the way
/// to keep turning past that, and they go through exactly the same
/// [`Head::turn`] the mouse does, as a rate rather than a per-frame delta.
const ARROW_LOOK_SPEED: f32 = 90.0;

impl Intent {
    fn read(input: &Input, dt: f32) -> Self {
        let arrows = Vec2::new(
            input.axis(Key::Left, Key::Right),
            input.axis(Key::Up, Key::Down),
        );
        let arrow_pixels =
            arrows * (ARROW_LOOK_SPEED.to_radians() * dt / camera::MOUSE_SENSITIVITY);
        Self {
            walk: Vec2::new(input.axis(Key::A, Key::D), input.axis(Key::S, Key::W)),
            look: input.mouse_delta() + arrow_pixels,
            sprint: input.key_down(Key::LeftShift) || input.key_down(Key::RightShift),
            jump: input.key_pressed(Key::Space),
            chop: input.key_pressed(Key::E) || input.mouse_pressed(MouseButton::Left),
            build: input.key_pressed(Key::B),
            tick_rate: input.key_pressed(Key::T),
        }
    }
}

// --- a recorded run -------------------------------------------------------

/// One stretch of a recorded run: an intent, and how long it is held.
struct Beat {
    seconds: f32,
    intent: Intent,
}

/// A recorded run, played back a frame at a time.
///
/// Held in seconds rather than frames so the same script drives the 60 Hz
/// window and the recording's much coarser frames to the same place. Mouse
/// movement is therefore a *rate* — pixels per second — and is multiplied by
/// the frame's own delta on the way out, which a raw per-frame delta could
/// never be.
pub struct Script {
    beats: Vec<Beat>,
    index: usize,
    held: f32,
    fresh: bool,
}

impl Script {
    fn new(beats: Vec<Beat>) -> Self {
        Self {
            beats,
            index: 0,
            held: 0.0,
            fresh: true,
        }
    }

    /// How long the whole run takes.
    pub fn duration(&self) -> f32 {
        self.beats.iter().map(|beat| beat.seconds).sum()
    }

    /// The intent for one frame, advancing the script by `dt`.
    ///
    /// A frame is not smaller than a beat — the recording's frames are longer
    /// than several of them — so `dt` is walked beat by beat rather than
    /// charged to whichever one the frame happened to start in. Two things
    /// depend on that, and both of them are invisible until the run comes out
    /// somewhere else:
    ///
    /// * **Turn is integrated.** `look` is a rate, and the part of the frame
    ///   spent in the next beat has to turn at *that* beat's rate, or a route
    ///   ends up pointing tens of degrees away from where it was written.
    /// * **No edge is skipped.** A frame that swallows a whole beat still
    ///   fires that beat's jump or swing of the axe; the levels are the ones
    ///   the frame began in, because a frame has to walk in one direction.
    fn frame(&mut self, dt: f32) -> Intent {
        let mut remaining = dt.max(0.0);
        let mut look = Vec2::ZERO;
        let mut levels: Option<Intent> = None;
        let mut edges = Intent::default();

        while remaining > 0.0 {
            let Some(beat) = self.beats.get(self.index) else {
                break;
            };
            if self.fresh {
                edges.jump |= beat.intent.jump;
                edges.chop |= beat.intent.chop;
                edges.build |= beat.intent.build;
                edges.tick_rate |= beat.intent.tick_rate;
            }
            levels.get_or_insert(beat.intent);
            self.fresh = false;

            let slice = remaining.min(beat.seconds - self.held);
            look = look + beat.intent.look * slice;
            self.held += slice;
            remaining -= slice;
            if self.held >= beat.seconds {
                self.held -= beat.seconds;
                self.index += 1;
                self.fresh = true;
            }
        }

        let levels = levels.unwrap_or_default();
        Intent {
            walk: levels.walk,
            sprint: levels.sprint,
            look,
            jump: edges.jump,
            chop: edges.chop,
            build: edges.build,
            tick_rate: edges.tick_rate,
        }
    }
}

/// Build one beat: `seconds` of `intent`.
fn beat(seconds: f32, intent: Intent) -> Beat {
    Beat { seconds, intent }
}

/// Walking forward, with the head turning at `degrees` per second.
///
/// A script says how fast to turn in degrees, not in mouse pixels: the
/// sensitivity is a setting and the route is not.
fn walking(degrees: f32) -> Intent {
    Intent {
        walk: Vec2::new(0.0, 1.0),
        look: Vec2::new(degrees.to_radians() / camera::MOUSE_SENSITIVITY, 0.0),
        ..Intent::default()
    }
}

/// Standing still while the head turns at `degrees` per second.
fn turning(degrees: f32) -> Intent {
    Intent {
        walk: Vec2::ZERO,
        ..walking(degrees)
    }
}

/// Standing still, turning at `degrees` per second and tipping the chin down
/// at `pitch` degrees per second. Screen `y` grows downwards, so looking down
/// is a *positive* mouse movement.
fn looking_down(degrees: f32, pitch: f32) -> Intent {
    let mut intent = turning(degrees);
    intent.look.y = pitch.to_radians() / camera::MOUSE_SENSITIVITY;
    intent
}

/// The recorded walkthrough: everything this card added, in one unbroken
/// forty-metre walk out of the camp and up to the rim.
///
/// Nothing in it is aimed at a moving target — the settlers wander where they
/// like and are walked *through*, not to — and no beat has to line up with a
/// frame: [`Script::frame`] integrates a frame across as many beats as it
/// covers, so the same seconds play out the same way in a window drawing
/// sixty frames a second and in a recording drawing two and a half.
pub fn walkthrough() -> Script {
    let still = Intent::default();
    let run = Intent {
        sprint: true,
        ..walking(0.0)
    };
    Script::new(vec![
        // Awake at the fire, looking up the valley.
        beat(0.34, still),
        // Out of the camp, between the trunks of the sparse forest.
        beat(1.70, run),
        // Through the settlers: they give way, and so does the player.
        beat(
            1.36,
            Intent {
                sprint: true,
                ..walking(6.0)
            },
        ),
        // Into the thicket by the stream, picking a line through it.
        beat(1.02, walking(-14.0)),
        beat(1.02, walking(10.0)),
        // Over the old log — 0.3 m, a step and not a wall.
        beat(1.36, run),
        // Up the rock, a step at a time, each one under the step height.
        beat(1.02, walking(0.0)),
        // Turn on top of the rock to face back down the valley — the rim
        // behind one shoulder, so the drop is seen against the forest rather
        // than against a hillside — and walk off the metre-high side of it.
        beat(0.68, turning(126.0)),
        beat(1.36, walking(0.0)),
        // A jump, once there is level ground under the feet again: a jump
        // asked for in mid-air is not saved up, and should not be.
        beat(0.68, walking(0.0)),
        beat(
            0.34,
            Intent {
                jump: true,
                ..still
            },
        ),
        beat(0.68, still),
        // Round the rock on its low side and down the valley a little.
        beat(0.68, turning(134.0)),
        beat(2.38, run),
        // Face south along the foot of the rim — mountain filling the right of
        // the screen, valley the left — and then run *sideways* into it. The
        // east half of that is up a slope past the forty degrees anything can
        // climb and goes nowhere, so the run becomes a walk along the hill
        // without the script asking it to.
        beat(0.34, turning(88.0)),
        beat(
            5.10,
            Intent {
                walk: Vec2::new(1.0, 0.25),
                sprint: true,
                ..still
            },
        ),
        // Stopped against the slope, turn back to the valley: the last tree at
        // the tree line is there, and so is a background to see it against.
        // The head goes down with the turn, because what there is to watch
        // ends up on the ground — a log is 0.3 m tall, and from two metres
        // away that is below the bottom of a level screen.
        beat(0.42, looking_down(-107.0, 52.0)),
        beat(
            0.42,
            Intent {
                chop: true,
                ..still
            },
        ),
        // Watch it swing down, away, and lie there as a log.
        beat(1.90, still),
    ])
}

/// Where a frame's [`Intent`] comes from.
enum Director {
    /// A person at the keyboard.
    Live,
    /// A recorded run — the walkthrough the card's video records.
    Recorded(Script),
}

// --- settlers -------------------------------------------------------------

const SETTLER_COUNT: usize = 12;
/// They are not in a hurry: a settler walks, and only the player runs.
const SETTLER_SPEED: f32 = 1.3;
/// How far from its own patch of camp a settler will wander.
const WANDER_RADIUS: f32 = 6.0;
/// Close enough to a target to call it reached.
const WANDER_ARRIVED: f32 = 0.9;
/// Ticks before a settler gives up on a target it cannot reach — which
/// happens, because nobody here finds a path around anything.
const WANDER_PATIENCE: u32 = 120;
const SETTLER_SEED: u64 = 0x5E77_1E12_0000_0041;

/// One of the twelve, as the game holds them.
struct Settler {
    entity: Entity,
    /// Where they were spawned; they wander around this, not the whole valley.
    home: Vec2,
    target: Vec2,
    patience: u32,
    height: f32,
    shirt: Color,
}

/// Height of settler `index`, spread evenly over the range a person comes in.
fn settler_height(index: usize) -> f32 {
    1.6 + 0.3 * index as f32 / (SETTLER_COUNT - 1) as f32
}

/// Shirt of settler `index`: the four muted tones of `07-look.md`, each in
/// three shades, which is twelve people you can tell apart at forty metres
/// without a nameplate over anyone's head.
fn settler_shirt(index: usize) -> Color {
    let tones = look::SHIRT_TONES;
    let shades = [0.7, 1.0, 1.35];
    tones[index % tones.len()].scale_rgb(shades[(index / tones.len()) % shades.len()])
}

/// Where settler `index` starts: four abreast across the way up the valley,
/// three rows deep, all of it far enough from the fire that waking up does not
/// mean waking up inside somebody.
fn settler_home(index: usize) -> Vec2 {
    let across = Vec2::new(ROUTE.y, -ROUTE.x);
    let column = (index % 4) as f32 - 1.5;
    let row = (index / 4) as f32;
    CAMPFIRE + ROUTE * (4.0 + row * 3.0 + column * 0.8) + across * (column * 3.0)
}

// --- the tick-rate switch -------------------------------------------------

/// The rates `T` cycles through. 20 Hz is the valley's own, and the other
/// three are there so the difference can be felt with a hand rather than read
/// in a document.
const TICK_RATES: [f32; 4] = [1.0 / 60.0, 1.0 / 30.0, 1.0 / 20.0, 1.0 / 15.0];
/// The index of 20 Hz in [`TICK_RATES`].
const DEFAULT_TICK_RATE: usize = 2;

// --- building -------------------------------------------------------------

/// A wall section: three metres long, half a metre thick, and tall enough
/// that nobody walks over it.
const WALL_HALF: Vec2 = Vec2::new(1.5, 0.25);
const WALL_TOP: f32 = 2.0;
/// How long it takes to be walked out of a wall that went up around you.
const EVICTION_TIME: f32 = 0.5;
/// A finger's width of clearance, so the body ends up outside rather than
/// exactly on the face.
const EVICTION_MARGIN: f32 = 0.02;

/// A body being walked out of a blocker that appeared around it.
///
/// `physics::step` resolves a new overlap in a single tick, which is correct
/// and instantaneous — from inside a wall that is a teleport. So for half a
/// second the game takes the body over instead and walks it out along the
/// nearest face itself, writing `position` after the tick has run. Nothing
/// about physics changes; it simply has nothing left to push out.
struct Eviction {
    entity: Entity,
    from: Vec2,
    to: Vec2,
    elapsed: f32,
}

// --- the debug top-down view ---------------------------------------------

/// Metres shown from the centre of the screen to its shorter edge.
const DEBUG_VIEW_RADIUS: f32 = 20.0;

/// A fixed-scale, straight-down view centred on `body` — not a free camera, so
/// it never tempts anyone into playing from above. Draws the static grid's
/// cells, every tree's footprint, the body itself, and a segment for each
/// contact it currently has.
fn draw_top_down_debug(framebuffer: &mut Framebuffer, grid: &StaticGrid, body: &Body) {
    let center = Vec2::new(body.position.x, body.position.z);
    let (width, height) = (framebuffer.width() as f32, framebuffer.height() as f32);
    framebuffer.clear(Color::rgb(0.08, 0.09, 0.10));
    let scale = (width.min(height) * 0.5 - 8.0) / DEBUG_VIEW_RADIUS;
    let to_screen = |p: Vec2| -> (f32, f32) {
        let rel = p - center;
        // North (+z) is drawn up the screen.
        (width * 0.5 + rel.x * scale, height * 0.5 - rel.y * scale)
    };

    let cell = grid.cell_size();
    let grid_color = Color::rgb(0.28, 0.30, 0.30);
    let first = (
        ((center.x - DEBUG_VIEW_RADIUS) / cell).floor() as i32,
        ((center.y - DEBUG_VIEW_RADIUS) / cell).floor() as i32,
    );
    let last = (
        ((center.x + DEBUG_VIEW_RADIUS) / cell).ceil() as i32,
        ((center.y + DEBUG_VIEW_RADIUS) / cell).ceil() as i32,
    );
    for cx in first.0..=last.0 {
        let x = cx as f32 * cell;
        let a = to_screen(Vec2::new(x, center.y - DEBUG_VIEW_RADIUS));
        let b = to_screen(Vec2::new(x, center.y + DEBUG_VIEW_RADIUS));
        debug::draw_line(framebuffer, a, b, grid_color);
    }
    for cz in first.1..=last.1 {
        let z = cz as f32 * cell;
        let a = to_screen(Vec2::new(center.x - DEBUG_VIEW_RADIUS, z));
        let b = to_screen(Vec2::new(center.x + DEBUG_VIEW_RADIUS, z));
        debug::draw_line(framebuffer, a, b, grid_color);
    }

    let tree_color = Color::rgb(0.35, 0.85, 0.40);
    let building_color = Color::rgb(0.55, 0.65, 0.95);
    for shape in grid.shapes() {
        let position = Vec2::new(shape.base.x, shape.base.z);
        if (position - center).length() > DEBUG_VIEW_RADIUS + 5.0 {
            continue;
        }
        match shape.blocker {
            Blocker::Cylinder { radius, .. } => {
                debug::draw_circle(
                    framebuffer,
                    to_screen(position),
                    radius * scale,
                    14,
                    tree_color,
                );
            }
            Blocker::Box { half, facing, .. } => {
                let corners = box_footprint(position, half, facing).map(to_screen);
                debug::draw_quad(framebuffer, corners, building_color);
            }
        }
    }

    let body_xz = Vec2::new(body.position.x, body.position.z);
    debug::draw_circle(
        framebuffer,
        to_screen(body_xz),
        body.radius * scale,
        20,
        Color::rgb(1.0, 0.9, 0.25),
    );

    let contact_color = Color::rgb(1.0, 0.25, 0.25);
    let mut nearby = Vec::new();
    grid.query(body_xz.x, body_xz.y, &mut nearby);
    for index in nearby {
        let shape = grid.shape(index);
        if let Some((normal, depth)) = shape.resolve(body_xz, body.radius) {
            let from = to_screen(body_xz);
            let to = to_screen(body_xz + normal * depth);
            debug::draw_line(framebuffer, from, to, contact_color);
        }
    }
}

/// The four corners of a [`Blocker::Box`] footprint, in order around the
/// rectangle.
fn box_footprint(position: Vec2, half: Vec2, facing: Vec2) -> [Vec2; 4] {
    let f = facing.normalized();
    let r = Vec2::new(-f.y, f.x);
    [
        position + f * half.x + r * half.y,
        position - f * half.x + r * half.y,
        position - f * half.x - r * half.y,
        position + f * half.x - r * half.y,
    ]
}

// --- drawing --------------------------------------------------------------

/// Grass on the basin floor.
const GRASS_COLOR: Color = Color::rgb(0.40, 0.48, 0.30);
/// Wet silt along the streambed.
const SILT_COLOR: Color = Color::rgb(0.33, 0.35, 0.26);
/// Bare rock, where the rim tips past anything that can be walked up.
const ROCK_COLOR: Color = Color::rgb(0.34, 0.33, 0.31);

/// The colour of the ground at one of its nodes.
///
/// A directional light on a basin floor is nearly the same brightness
/// everywhere — the slopes here are one in ten — so without this the whole
/// valley is one flat green field and nothing in it reads as a shape. The two
/// things the ground is actually made of are the two things that decide the
/// colour: how steep it is, and how deep in the streambed.
fn ground_color(position: Vec3, normal: Vec3) -> Color {
    let steep = ((1.0 - normal.y) / (1.0 - SLOPE_40_DEGREES_COS)).clamp(0.0, 1.0);
    let wet = (1.0 - (position.z / STREAM_HALF_WIDTH).abs()).clamp(0.0, 1.0);
    let ground = GRASS_COLOR
        .lerp(SILT_COLOR, wet * 0.8)
        .lerp(ROCK_COLOR, steep);
    // A hillside lit by one directional light and painted one colour is a
    // silhouette and nothing else. A few percent of variation per node, hashed
    // off the node's own coordinates so it never crawls, is enough to see
    // which way the ground runs.
    let shade = 1.0 + 0.09 * (node_noise(position) - 0.5);
    ground.scale_rgb(shade)
}

/// Deterministic value noise in `[0, 1)` for a world position — the sine hash
/// `valley/look.rs` uses for its embers, in two dimensions.
fn node_noise(position: Vec3) -> f32 {
    let x = (position.x * 12.9898 + position.z * 78.233).sin() * 43_758.547;
    x - x.floor()
}

/// Paint a terrain mesh's nodes with [`ground_color`]. The mesh is still one
/// draw call; the colour rides on the vertices.
fn paint_terrain(mut mesh: Mesh) -> Mesh {
    for vertex in &mut mesh.vertices {
        vertex.color = ground_color(vertex.position, vertex.normal);
    }
    mesh
}

/// How far the air stays clear before the sky starts eating the colour out of
/// the hillside. Without it the rim is a flat wall of rock two hundred metres
/// away and reads as if it were ten.
const FOG_START: f32 = 25.0;
const FOG_END: f32 = 420.0;

/// How far away a trunk or a settler is still drawn. The valley is half a
/// kilometre across and the eye is 1.6 m off the ground; past this the trunks
/// are a pixel wide and cost more than they show.
const DRAW_DISTANCE: f32 = 90.0;

/// Model matrix for a trunk of `height` and `radius` standing at `base` and
/// leaning as `tree` says.
///
/// The lean is a `(sin, cos)` pair and the fall has a compass direction, which
/// is already a basis: up the trunk, across it, and along the ground. No angle
/// is ever formed, on this side of the fence either.
fn trunk_matrix(base: Vec3, height: f32, radius: f32, tree: &FallingTree) -> Mat4 {
    let dir = tree.direction;
    let axis = Vec3::new(dir.x * tree.lean.x, tree.lean.y, dir.y * tree.lean.x);
    let across = Vec3::new(-dir.y, 0.0, dir.x);
    let along = across.cross(axis);
    // Flat on the ground, the trunk's axis has to drop to where a log's top is
    // the 0.3 m step `physics::step` gives it, not a 0.7 m barrel.
    let sink = (LOG_TOP - radius) * tree.lean.x;
    Mat4::from_cols(
        (across * radius).extend(0.0),
        (axis * height).extend(0.0),
        (along * radius).extend(0.0),
        Vec3::new(base.x, base.y + sink, base.z).extend(1.0),
    )
}

/// Model matrix for a [`Blocker::Box`] standing on `base`: a unit cube scaled
/// to the footprint and the height, turned to face `facing`.
fn box_matrix(base: Vec3, half: Vec2, facing: Vec2, top: f32) -> Mat4 {
    let f = facing.normalized();
    let r = Vec2::new(-f.y, f.x);
    Mat4::from_cols(
        (Vec3::new(f.x, 0.0, f.y) * (half.x * 2.0)).extend(0.0),
        (Vec3::Y * top).extend(0.0),
        (Vec3::new(r.x, 0.0, r.y) * (half.y * 2.0)).extend(0.0),
        Vec3::new(base.x, base.y + top * 0.5, base.z).extend(1.0),
    )
}

// --- the game -------------------------------------------------------------

/// The hour the valley is played at: mid-morning, sun well up.
const HOUR: f32 = 9.0;

/// The longest frame the clock will admit to, as a guard against a stall.
///
/// The recorded run draws two and a half frames a second so that sixty of them
/// cover the whole walk, which is far longer than any frame a window produces;
/// this has to stay above [`RECORD_FRAME_DELTA`] or the recording would
/// quietly play the script at the wrong speed and stop short of its own end.
const MAX_FRAME_DELTA: f32 = 0.5;

pub struct Valley {
    far_mesh: Mesh,
    patch_mesh: Mesh,
    patch_center: Vec2,
    tree_mesh: Mesh,
    body_mesh: Mesh,
    box_mesh: Mesh,

    fog: Fog,
    physics: PhysicsWorld,
    trees: Vec<Tree>,
    standing: Vec<Standing>,
    settlers: Vec<Settler>,
    logs: Vec<Log>,
    walls: Vec<Wall>,
    steps: Vec<Step>,
    evictions: Vec<Eviction>,

    player: Option<Entity>,
    head: Head,
    stamina: Stamina,
    director: Director,
    tick_rate: usize,
    top_down: bool,
    font: Font,
    hud: bool,
}

impl Valley {
    /// The valley with a person at the keyboard in it.
    pub fn live() -> Self {
        Self::with_director(Director::Live)
    }

    /// The same valley, driven by a recorded run instead of a keyboard.
    pub fn recorded(script: Script) -> Self {
        Self::with_director(Director::Recorded(script))
    }

    fn with_director(director: Director) -> Self {
        let trees = place_forest(FOREST_SEED);
        let ground = build_physics_heightfield();
        let start = snap_patch(CAMPFIRE);
        let physics = PhysicsWorld::new(ground, Tuning::default(), TICK_RATES[DEFAULT_TICK_RATE]);
        let far = build_far_heightfield();
        let patch = build_patch_heightfield(CAMPFIRE);
        let feet = Vec3::new(
            CAMPFIRE.x,
            physics.ground.height_at(CAMPFIRE.x, CAMPFIRE.y),
            CAMPFIRE.y,
        );

        let (_, sky) = look::sky_at(HOUR);
        Self {
            far_mesh: paint_terrain(far.to_mesh_lod(FAR_LOD_STRIDE)),
            patch_mesh: paint_terrain(patch.to_mesh()),
            patch_center: start,
            tree_mesh: Mesh::cylinder(1.0, 1.0, 8),
            body_mesh: Mesh::cylinder(1.0, 1.0, 10),
            box_mesh: Mesh::cube(1.0),

            fog: Fog {
                color: sky,
                start: FOG_START,
                end: FOG_END,
            },
            physics,
            trees,
            standing: Vec::new(),
            settlers: Vec::new(),
            logs: Vec::new(),
            walls: Vec::new(),
            steps: Vec::new(),
            evictions: Vec::new(),

            player: None,
            // Looking up the route out of the camp, which runs along the rim
            // rather than at it.
            head: Head::new(feet, route_yaw()),
            stamina: Stamina::full(),
            director,
            tick_rate: DEFAULT_TICK_RATE,
            top_down: false,
            font: Font::embedded(),
            hud: true,
        }
    }

    /// Everything that lives in the world: the forest, the camp's own props,
    /// the twelve, and the player.
    fn populate(&mut self, world: &mut World) {
        self.standing = populate_world(world, &self.physics.ground, &self.trees);

        let log_base = Vec3::new(
            OLD_LOG.x,
            self.physics.ground.height_at(OLD_LOG.x, OLD_LOG.y),
            OLD_LOG.y,
        );
        self.spawn_blocker(
            world,
            log_base,
            Blocker::Box {
                half: Vec2::new(OLD_LOG_LENGTH * 0.5, TREE_RADIUS),
                facing: OLD_LOG_FACING,
                top: LOG_TOP,
            },
        );
        self.logs.push(Log {
            base: log_base,
            facing: OLD_LOG_FACING,
            length: OLD_LOG_LENGTH,
        });

        // Every step of the outcrop stands on one base — the ground it is
        // walked onto from — so each riser is the height it looks and not that
        // plus whatever the hillside climbed in between.
        let approach = OUTCROP - OUTCROP_FACING * (3.0 * OUTCROP_HALF.x + 0.4);
        let rock_base = Vec3::new(
            OUTCROP.x,
            self.physics.ground.height_at(approach.x, approach.y),
            OUTCROP.y,
        );
        for (index, top) in OUTCROP_TOPS.into_iter().enumerate() {
            let offset = OUTCROP_FACING * ((index as f32 - 1.0) * OUTCROP_HALF.x * 2.0);
            let base = Vec3::new(rock_base.x + offset.x, rock_base.y, rock_base.z + offset.y);
            self.spawn_blocker(
                world,
                base,
                Blocker::Box {
                    half: OUTCROP_HALF,
                    facing: OUTCROP_FACING,
                    top,
                },
            );
            self.steps.push(Step {
                base,
                half: OUTCROP_HALF,
                facing: OUTCROP_FACING,
                top,
            });
        }

        for index in 0..SETTLER_COUNT {
            let home = settler_home(index);
            let height = settler_height(index);
            let entity = world.spawn();
            let feet = Vec3::new(
                home.x,
                self.physics.ground.height_at(home.x, home.y),
                home.y,
            );
            world.insert(entity, Body::new(feet).with_size(0.3, height));
            world.insert(entity, PendingInput::new());
            self.settlers.push(Settler {
                entity,
                home,
                target: home,
                patience: 0,
                height,
                shirt: settler_shirt(index),
            });
        }

        let feet = Vec3::new(
            CAMPFIRE.x,
            self.physics.ground.height_at(CAMPFIRE.x, CAMPFIRE.y),
            CAMPFIRE.y,
        );
        let player = world.spawn();
        world.insert(player, Body::new(feet));
        world.insert(player, PendingInput::new());
        self.player = Some(player);
    }

    /// The player's entity. Every method that wants it runs after
    /// [`Game::start`], which is where it is spawned.
    fn player(&self) -> Entity {
        self.player.expect("the player is spawned in start()")
    }

    fn spawn_blocker(&mut self, world: &mut World, base: Vec3, blocker: Blocker) -> Entity {
        let entity = world.spawn();
        world.insert(entity, Transform::from_position(base));
        world.insert(entity, blocker);
        self.physics.mark_statics_dirty();
        entity
    }

    fn body(&self, world: &World, entity: Entity) -> Body {
        *world
            .get::<Body>(entity)
            .expect("every body this game spawned still has one")
    }

    // --- one frame --------------------------------------------------------

    /// Turn a frame's intent into commands, a heading, and whatever the two
    /// action keys did.
    fn act(&mut self, intent: Intent, engine: &mut Engine) {
        self.head.turn(intent.look);

        if intent.tick_rate {
            self.cycle_tick_rate(engine);
        }

        let direction = self.head.ahead() * intent.walk.y + self.head.right() * intent.walk.x;
        let moving = direction.length() > 1e-3;
        let running = moving && intent.sprint;
        self.stamina.breathe(engine.time.delta(), running, moving);

        let jumping = intent.jump && self.stamina.can_jump();
        if let Some(pending) = engine.world.get_mut::<PendingInput>(self.player()) {
            pending.push(Command::Move {
                direction,
                speed_scale: self.stamina.speed_scale(intent.sprint),
            });
            if jumping {
                pending.push(Command::Jump);
            }
        }
        if jumping {
            self.stamina.spend_jump();
        }

        if intent.chop {
            self.swing_the_axe(engine);
        }
        if intent.build {
            self.build_a_wall(engine);
        }
    }

    /// Move the whole simulation — clock and physics together — onto the next
    /// rate. Doing one without the other is how a world quietly runs fast.
    fn cycle_tick_rate(&mut self, engine: &mut Engine) {
        self.tick_rate = (self.tick_rate + 1) % TICK_RATES.len();
        let delta = TICK_RATES[self.tick_rate];
        engine.time.fixed_delta = delta;
        self.physics.set_fixed_delta(delta);
    }

    /// Hertz of the rate currently running, for a window title.
    fn tick_hz(&self) -> f32 {
        1.0 / TICK_RATES[self.tick_rate]
    }

    /// Fell the nearest trunk within reach, away from the player.
    ///
    /// Returns the tree that was hit, if any — the fall itself is
    /// `physics::step`'s business from here: the pendulum, the shove it gives
    /// anything underneath, and the log it leaves behind.
    fn swing_the_axe(&mut self, engine: &mut Engine) -> Option<Entity> {
        let body = self.body(&engine.world, self.player());
        let from = Vec2::new(body.position.x, body.position.z);
        let ahead = self.head.ahead();
        let ahead = Vec2::new(ahead.x, ahead.z);

        let mut best: Option<(f32, usize)> = None;
        for (index, tree) in self.standing.iter().enumerate() {
            let delta = tree.position - from;
            let distance = delta.length();
            if distance > AXE_REACH {
                continue;
            }
            // Behind you is not within reach of a swing.
            if delta.dot(ahead) <= 0.0 {
                continue;
            }
            let standing = engine
                .world
                .get::<FallingTree>(tree.entity)
                .is_some_and(|t| !t.falling && !t.fallen);
            if !standing {
                continue;
            }
            if best.map_or(true, |(best_distance, _)| distance < best_distance) {
                best = Some((distance, index));
            }
        }

        let (_, index) = best?;
        let tree = &self.standing[index];
        let away = (tree.position - from).normalized();
        let away = if away == Vec2::ZERO { ahead } else { away };
        engine
            .world
            .get_mut::<FallingTree>(tree.entity)?
            .topple(away);
        Some(tree.entity)
    }

    /// Put a wall down where the player stands, and start walking out of it
    /// whoever that just enclosed — the player included, which is nearly
    /// always the case.
    fn build_a_wall(&mut self, engine: &mut Engine) {
        let body = self.body(&engine.world, self.player());
        let base = body.position;
        let facing = self.head.right();
        let facing = Vec2::new(facing.x, facing.z).normalized();
        let blocker = Blocker::Box {
            half: WALL_HALF,
            facing,
            top: WALL_TOP,
        };
        let entity = self.spawn_blocker(&mut engine.world, base, blocker);
        self.walls.push(Wall {
            base,
            half: WALL_HALF,
            facing,
            top: WALL_TOP,
        });

        let shape = StaticShape {
            entity,
            base,
            blocker,
        };
        let inside: Vec<Entity> = std::iter::once(self.player())
            .chain(self.settlers.iter().map(|s| s.entity))
            .collect();
        for entity in inside {
            let body = self.body(&engine.world, entity);
            let point = Vec2::new(body.position.x, body.position.z);
            let Some((normal, depth)) = shape.resolve(point, body.radius) else {
                continue;
            };
            self.evictions.retain(|e| e.entity != entity);
            self.evictions.push(Eviction {
                entity,
                from: point,
                to: point + normal * (depth + EVICTION_MARGIN),
                elapsed: 0.0,
            });
        }
    }

    /// Steer the twelve. No paths are found: a settler walks at its target and
    /// slides along whatever it meets, which is the whole of its navigation.
    fn steer_settlers(&mut self, engine: &mut Engine) {
        let tick = engine.time.elapsed_ticks();
        for (index, settler) in self.settlers.iter_mut().enumerate() {
            let Some(body) = engine.world.get::<Body>(settler.entity) else {
                continue;
            };
            let here = Vec2::new(body.position.x, body.position.z);
            let delta = settler.target - here;
            if delta.length() < WANDER_ARRIVED || settler.patience == 0 {
                let mut rng = TickRng::new(SETTLER_SEED ^ index as u64, tick);
                let angle = rng.next_f32() * std::f32::consts::TAU;
                let radius = WANDER_RADIUS * rng.next_f32().sqrt();
                settler.target = settler.home + Vec2::new(angle.cos(), angle.sin()) * radius;
                settler.patience = WANDER_PATIENCE;
            }
            settler.patience -= 1;
            if let Some(pending) = engine.world.get_mut::<PendingInput>(settler.entity) {
                pending.push(Command::Move {
                    direction: Vec3::new(delta.x, 0.0, delta.y).normalized(),
                    speed_scale: SETTLER_SPEED / RUN_SPEED,
                });
            }
        }
    }

    /// Walk anyone a new wall enclosed out of it, a tick at a time.
    ///
    /// Runs *after* the tick, so what it writes is what the frame draws, and
    /// it writes `prev_position`'s partner rather than fighting with it: the
    /// tick set `prev_position` to last tick's sample, and this sets
    /// `position` to this one, so the interpolated path is exactly the smooth
    /// curve below.
    fn settle_evictions(&mut self, engine: &mut Engine, dt: f32) {
        for eviction in &mut self.evictions {
            eviction.elapsed += dt;
            let t = (eviction.elapsed / EVICTION_TIME).clamp(0.0, 1.0);
            // Smoothstep: it starts and ends at a standstill, so neither end
            // of the half-second reads as a shove.
            let eased = t * t * (3.0 - 2.0 * t);
            let point = eviction.from + (eviction.to - eviction.from) * eased;
            if let Some(body) = engine.world.get_mut::<Body>(eviction.entity) {
                body.position.x = point.x;
                body.position.z = point.y;
                body.velocity.x = 0.0;
                body.velocity.z = 0.0;
            }
        }
        self.evictions.retain(|e| e.elapsed < EVICTION_TIME);
    }

    /// Follow the body with the head, on this frame's `alpha`.
    fn track_head(&mut self, engine: &mut Engine) {
        let body = self.body(&engine.world, self.player());
        let feet = body.render_position(engine.time.fixed_alpha());
        self.head.follow(feet, body.grounded, engine.time.delta());
        self.head.aim(&mut engine.camera);

        let center = snap_patch(Vec2::new(feet.x, feet.z));
        if center != self.patch_center {
            self.patch_center = center;
            self.patch_mesh = paint_terrain(build_patch_heightfield(center).to_mesh());
        }
    }

    /// A lit shader for this scene: the frame's camera and light, plus the
    /// valley's own haze, which every draw has to agree on.
    fn scene_shader(&self, engine: &Engine, model: Mat4, color: Color) -> BasicShader<'static> {
        let mut shader = engine
            .lit_shader(model)
            .with_base_color(color)
            .with_fog(self.fog);
        // Grass, bark and rock are not wet: the default quarter of a specular
        // highlight turns a whole hillside into one pale sheet.
        shader.specular_strength = 0.03;
        shader
    }

    fn draw_scene(&self, engine: &mut Engine, alpha: f32) {
        let eye = Vec2::new(self.head.eye().x, self.head.eye().z);

        // White base colour: the ground's own colour is on its vertices.
        let ground = self.scene_shader(engine, Mat4::IDENTITY, Color::WHITE);
        engine.draw(&self.far_mesh, &ground);
        engine.draw(&self.patch_mesh, &ground);

        let trunk_color = Color::rgb(0.30, 0.22, 0.14);
        for tree in &self.standing {
            if (tree.position - eye).length() > DRAW_DISTANCE {
                continue;
            }
            let Some(state) = engine.world.get::<FallingTree>(tree.entity).copied() else {
                continue;
            };
            let model = trunk_matrix(tree.base, tree.height, TREE_RADIUS, &state);
            let shader = self.scene_shader(engine, model, trunk_color);
            engine.draw(&self.tree_mesh, &shader);
        }

        for log in &self.logs {
            let mut lying = FallingTree::new(log.length, TREE_RADIUS);
            lying.lean = Vec2::new(1.0, 0.0);
            lying.direction = log.facing.normalized();
            let model = trunk_matrix(log.base, log.length, TREE_RADIUS, &lying);
            let shader = self.scene_shader(engine, model, trunk_color);
            engine.draw(&self.tree_mesh, &shader);
        }

        let rock_color = Color::rgb(0.42, 0.42, 0.40);
        for step in &self.steps {
            let model = box_matrix(step.base, step.half, step.facing, step.top);
            let shader = self.scene_shader(engine, model, rock_color);
            engine.draw(&self.box_mesh, &shader);
        }

        let wall_color = Color::rgb(0.52, 0.45, 0.36);
        for wall in &self.walls {
            let model = box_matrix(wall.base, wall.half, wall.facing, wall.top);
            let shader = self.scene_shader(engine, model, wall_color);
            engine.draw(&self.box_mesh, &shader);
        }

        for settler in &self.settlers {
            let Some(body) = engine.world.get::<Body>(settler.entity).copied() else {
                continue;
            };
            let feet = body.render_position(alpha);
            if (Vec2::new(feet.x, feet.z) - eye).length() > DRAW_DISTANCE {
                continue;
            }
            let model = Mat4::from_translation(feet)
                * Mat4::from_scale(Vec3::new(body.radius, settler.height, body.radius));
            let shader = self.scene_shader(engine, model, settler.shirt);
            engine.draw(&self.body_mesh, &shader);
        }

        // The fire the player woke up next to: embers, the one saturated
        // colour the valley's palette allows (`07-look.md`).
        let fire = Vec3::new(
            CAMPFIRE.x,
            self.physics.ground.height_at(CAMPFIRE.x, CAMPFIRE.y),
            CAMPFIRE.y,
        );
        let model = Mat4::from_translation(fire) * Mat4::from_scale(Vec3::new(0.6, 0.25, 0.6));
        let shader = self.scene_shader(engine, model, look::EMBER_COLOR);
        engine.draw(&self.body_mesh, &shader);
    }

    // --- the heads-up display ---------------------------------------------

    /// What the top-left panel says. Every number in it is one this card put
    /// there and nothing else can show: the tick rate `T` is cycling, how much
    /// breath the run has left, and whether the feet are on the ground.
    fn hud_status(&self, engine: &Engine) -> String {
        format!(
            "такт: {:.0} Гц   кадров/с: {:.0}\nусталость: {:.0}%   {}\nвид: {}",
            self.tick_hz(),
            engine.time.average_fps(),
            self.stamina.remaining() * 100.0,
            if self.grounded(&engine.world) {
                "на земле"
            } else {
                "в воздухе"
            },
            if self.top_down {
                "сверху (M)"
            } else {
                "от первого лица (M)"
            },
        )
    }

    /// Whether the player's feet are on something, for the panel.
    fn grounded(&self, world: &World) -> bool {
        world
            .get::<Body>(self.player())
            .is_some_and(|body| body.grounded)
    }

    /// The status panel, the breath bar under it, and the key list along the
    /// bottom — so that everything this card added can be reached by somebody
    /// who has not read the source.
    fn draw_hud(&self, engine: &mut Engine) {
        let status = self.hud_status(engine);
        let style = TextStyle::new(&self.font)
            .size(HUD_TEXT_SIZE)
            .color(Color::WHITE)
            .tabular_digits(true)
            .panel(Color::rgba(0.0, 0.0, 0.0, HUD_PANEL_ALPHA));
        let size = style.measure(&status);
        style.draw(&mut engine.framebuffer, &status, HUD_MARGIN, HUD_MARGIN);

        // The breath the sprint is spending, as a bar: the number above says
        // the same thing, but the bar is what is readable while running.
        let bar_y = HUD_MARGIN + size.height.round() as i32 + 4;
        let filled = (STAMINA_BAR_WIDTH as f32 * self.stamina.remaining()).round() as usize;
        engine.framebuffer.fill_rect(
            HUD_MARGIN,
            bar_y,
            STAMINA_BAR_WIDTH,
            STAMINA_BAR_HEIGHT,
            Color::rgba(0.0, 0.0, 0.0, HUD_PANEL_ALPHA),
        );
        engine.framebuffer.fill_rect(
            HUD_MARGIN,
            bar_y,
            filled,
            STAMINA_BAR_HEIGHT,
            look::EMBER_COLOR,
        );

        let keys = KEY_HELP
            .iter()
            .map(|(key, action)| format!("{key}\t{action}"))
            .collect::<Vec<_>>()
            .join("\n");
        let size = style.measure(&keys);
        let y = engine.framebuffer.height() as i32 - size.height.round() as i32 - HUD_MARGIN;
        style.draw(&mut engine.framebuffer, &keys, HUD_MARGIN, y);
    }
}

/// Pixel size of every panel, and how opaque the black behind it is.
const HUD_TEXT_SIZE: f32 = 15.0;
const HUD_PANEL_ALPHA: f32 = 0.55;
/// Distance of a panel from the edge of the frame.
const HUD_MARGIN: i32 = 8;
/// The breath bar under the status panel.
const STAMINA_BAR_WIDTH: usize = 160;
const STAMINA_BAR_HEIGHT: usize = 6;

/// The keys, as `F` shows them — the same list as this file's module comment.
const KEY_HELP: &[(&str, &str)] = &[
    ("WASD", "идти; Shift — бежать, Space — прыгать"),
    ("мышь / стрелки", "крутить голову"),
    ("E", "рубить ближайшее дерево"),
    ("B", "поставить стену (и выйти из неё)"),
    ("T", "такт 60 / 30 / 20 / 15 Гц, вживую"),
    ("M", "вид сверху на препятствия"),
    ("F", "убрать или вернуть этот текст"),
    ("Escape", "выход"),
];

impl Game for Valley {
    fn start(&mut self, engine: &mut Engine) -> io::Result<()> {
        let (light, sky) = look::sky_at(HOUR);
        engine.light = light;
        engine.clear_color = sky;
        engine.ambient = Color::rgb(0.24, 0.26, 0.30);
        engine.time.fixed_delta = TICK_RATES[self.tick_rate];
        engine.time.max_delta = MAX_FRAME_DELTA;
        // The chunk is 512 m across and the rim is visible from anywhere in
        // it; the default 500 m far plane would cut it off.
        engine.camera.far = 900.0;
        self.populate(&mut engine.world);
        self.track_head(engine);
        Ok(())
    }

    fn update(&mut self, engine: &mut Engine) {
        if engine.input.key_pressed(Key::Escape) {
            engine.quit();
        }
        if engine.input.key_pressed(Key::M) {
            self.top_down = !self.top_down;
        }
        if engine.input.key_pressed(Key::F) {
            self.hud = !self.hud;
        }
        let dt = engine.time.delta();
        let intent = match &mut self.director {
            Director::Live => Intent::read(&engine.input, dt),
            Director::Recorded(script) => script.frame(dt),
        };
        self.act(intent, engine);
        engine.set_title(format!(
            "runity — valley ({:.0} Hz tick, T to change)",
            self.tick_hz()
        ));
    }

    fn fixed_update(&mut self, engine: &mut Engine) {
        self.steer_settlers(engine);
        // Only the player jumps here, so the tuning's jump is the player's.
        self.physics.tuning.jump_speed = self.stamina.jump_speed();
        self.physics.step(&mut engine.world);
        self.settle_evictions(engine, self.physics.fixed_delta());
    }

    fn render(&mut self, engine: &mut Engine) {
        self.track_head(engine);
        let alpha = engine.time.fixed_alpha();
        if self.top_down {
            let body = self.body(&engine.world, self.player());
            draw_top_down_debug(&mut engine.framebuffer, self.physics.statics(), &body);
        } else {
            self.draw_scene(engine, alpha);
        }
        // The recorded run is the card's video and wants a clean picture; a
        // person at the keyboard wants to know which keys there are.
        if self.hud && matches!(self.director, Director::Live) {
            self.draw_hud(engine);
        }
    }
}

fn main() -> io::Result<()> {
    let config = WindowConfig::new("runity — valley", WIDTH, HEIGHT);
    let headless = std::env::var("RUNITY_HEADLESS").is_ok();
    let recorded = std::env::var("RUNITY_WALKTHROUGH").is_ok();

    if std::env::var("RUNITY_RECORD").is_ok() {
        return record_the_walkthrough();
    }

    let mut app = App::new(config);
    if headless {
        app = app
            .with_max_frames(1)
            .with_frame_delta(1.0 / 60.0)
            .with_target_fps(None);
    }

    let engine = if recorded {
        let script = walkthrough();
        println!(
            "playing the recorded walkthrough: {:.1} s",
            script.duration()
        );
        app.run(Valley::recorded(script))?
    } else {
        app.run(Valley::live())?
    };

    if headless {
        let path = std::env::var("RUNITY_SCREENSHOT").unwrap_or_else(|_| "valley.png".to_string());
        save_png(&path, &engine.framebuffer)?;
        println!(
            "wrote {path} ({}x{})",
            engine.framebuffer.width(),
            engine.framebuffer.height()
        );
    }
    Ok(())
}

// --- the recording --------------------------------------------------------

/// Frames the card's video is, and how long each one covers.
///
/// The tick stays the valley's own 20 Hz, and the frame delta is deliberately
/// not a whole number of ticks: nine frames in ten are drawn *between* two
/// ticks, at a different `alpha` each time, which is where interpolation done
/// badly shows up as a pulse instead of as motion.
const RECORD_FRAMES: u64 = 60;
const RECORD_FRAME_DELTA: f32 = 0.38;
const RECORD_WIDTH: u32 = 640;
const RECORD_HEIGHT: u32 = 360;

/// Where `RUNITY_RECORD=1` writes its PNGs.
fn record_directory() -> std::path::PathBuf {
    std::env::var_os("STUDIO_ARTIFACTS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// Play [`walkthrough`] with no window and keep every frame.
///
/// One unbroken walk: out of the camp, up the slope between the trunks,
/// through the twelve, into the thicket by the stream, over the old log, up
/// the outcrop and off the side of it, a jump on level ground, into the rim
/// and along it, and a tree felled at the tree line.
fn record_the_walkthrough() -> io::Result<()> {
    let out = record_directory();
    let script = walkthrough();
    println!(
        "recording {RECORD_FRAMES} frames of a {:.1} s walk at {:.0} Hz tick",
        script.duration(),
        1.0 / TICK_RATES[DEFAULT_TICK_RATE]
    );
    let frames = headless::record(
        Valley::recorded(script),
        RECORD_WIDTH,
        RECORD_HEIGHT,
        RECORD_FRAMES,
        RECORD_FRAME_DELTA,
    )?;
    let written = headless::save_frames(&out, "walk-", &frames)?;
    println!("wrote {} frames into {}", written.len(), out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- terrain -----------------------------------------------------

    #[test]
    fn the_rim_reaches_the_forty_degree_limit() {
        let far = build_far_heightfield();
        let steep = (0..200)
            .map(|i| INNER_RADIUS + 1.0 + i as f32 * 0.3)
            .any(|r| far.normal_at(r, 0.0).y < SLOPE_40_DEGREES_COS);
        assert!(
            steep,
            "no radius along the rim is steep enough to block a 40 degree climb"
        );
    }

    #[test]
    fn the_forest_area_stays_within_the_walkable_slope() {
        let far = build_far_heightfield();
        for &(x, z) in &[
            (0.0, 0.0),
            (100.0, 0.0),
            (-150.0, 80.0),
            (0.0, 180.0),
            (60.0, 5.0),
            (CAMPFIRE.x, CAMPFIRE.y),
        ] {
            let normal = far.normal_at(x, z);
            assert!(
                normal.y >= SLOPE_40_DEGREES_COS,
                "({x}, {z}) inside the forest is too steep to stand on: {normal:?}"
            );
        }
    }

    #[test]
    fn the_patch_edge_matches_nodes_the_far_ring_already_kept() {
        let far = build_far_heightfield();
        let patch = build_patch_heightfield(Vec2::ZERO);
        let far_mesh = far.to_mesh_lod(FAR_LOD_STRIDE);
        let patch_mesh = patch.to_mesh();

        let height_at = |mesh: &Mesh, x: f32, z: f32| -> f32 {
            mesh.vertices
                .iter()
                .find(|v| (v.position.x - x).abs() < 1e-3 && (v.position.z - z).abs() < 1e-3)
                .unwrap_or_else(|| panic!("no vertex at ({x}, {z})"))
                .position
                .y
        };

        for &fixed in &[-32.0f32, 32.0] {
            for &varying in &[-32.0f32, -16.0, 0.0, 16.0, 32.0] {
                assert_eq!(
                    height_at(&far_mesh, fixed, varying),
                    height_at(&patch_mesh, fixed, varying),
                    "seam at x={fixed}, z={varying}"
                );
                assert_eq!(
                    height_at(&far_mesh, varying, fixed),
                    height_at(&patch_mesh, varying, fixed),
                    "seam at x={varying}, z={fixed}"
                );
            }
        }
    }

    #[test]
    fn physics_walks_on_the_same_surface_the_patch_draws() {
        // A foot inside a hillside is what a coarser physics ground buys: the
        // patch under the player and the field it stands on are sampled at the
        // same resolution, on the same lattice, from the same formula.
        let ground = build_physics_heightfield();
        for step in 0..40 {
            let x = CAMPFIRE.x + step as f32 * 0.5;
            let z = CAMPFIRE.y + step as f32 * 0.17;
            assert!(
                (ground.height_at(x, z) - terrain_height(x, z)).abs() < 0.05,
                "({x}, {z}): physics {} against the formula {}",
                ground.height_at(x, z),
                terrain_height(x, z)
            );
        }
    }

    // --- forest --------------------------------------------------------

    #[test]
    fn no_two_trees_stand_closer_than_the_minimum_clearance() {
        let trees = place_forest(FOREST_SEED);
        assert!(
            trees.len() > 100,
            "sanity: the forest actually placed trees"
        );
        for i in 0..trees.len() {
            for j in (i + 1)..trees.len() {
                let distance = (trees[i].position - trees[j].position).length();
                // The camp's thicket is deliberately tighter than the forest;
                // everything still has to leave a body room to pass.
                let close_to_camp = (trees[i].position - THICKET_CENTER).length()
                    < THICKET_RADIUS + MIN_TREE_SPACING
                    || (trees[j].position - THICKET_CENTER).length()
                        < THICKET_RADIUS + MIN_TREE_SPACING;
                let floor = if close_to_camp {
                    THICKET_SPACING
                } else {
                    MIN_TREE_SPACING
                };
                assert!(
                    distance >= floor - 1e-3,
                    "trees {i} and {j} are only {distance} m apart"
                );
            }
        }
    }

    #[test]
    fn a_body_fits_between_the_thicket_trunks() {
        // 2.2 m between axes, 0.35 m of trunk on each side, 0.3 m of body: a
        // gap a person can walk through, if they aim.
        let clearance =
            THICKET_SPACING - 2.0 * TREE_RADIUS - 2.0 * Tuning::default().min_body_radius;
        assert!(clearance > 0.2, "only {clearance} m of gap");
    }

    #[test]
    fn the_sparse_forest_is_close_to_its_target_density() {
        let trees = place_forest(FOREST_SEED);
        let sparse_area = std::f32::consts::PI * FOREST_RADIUS * FOREST_RADIUS
            - std::f32::consts::PI * DENSE_PATCH_RADIUS * DENSE_PATCH_RADIUS;
        let expected = sparse_area * BASE_TREE_DENSITY;
        let sparse_count = trees
            .iter()
            .filter(|t| (t.position - DENSE_PATCH_CENTER).length() > DENSE_PATCH_RADIUS)
            .filter(|t| (t.position - THICKET_CENTER).length() > THICKET_RADIUS)
            .filter(|t| t.position != LONE_TREE)
            .count() as f32;
        assert!(
            (sparse_count - expected).abs() < expected * 0.2,
            "sparse count {sparse_count} vs target {expected}"
        );
    }

    #[test]
    fn the_dense_patch_is_markedly_denser_than_the_rest_of_the_forest() {
        let trees = place_forest(FOREST_SEED);
        let dense_area = std::f32::consts::PI * DENSE_PATCH_RADIUS * DENSE_PATCH_RADIUS;
        let dense_count = trees
            .iter()
            .filter(|t| (t.position - DENSE_PATCH_CENTER).length() <= DENSE_PATCH_RADIUS)
            .count();
        let dense_density = dense_count as f32 / dense_area;
        assert!(
            dense_density > BASE_TREE_DENSITY * (DENSE_DENSITY_MULT - 1.0),
            "the grove isn't noticeably denser than the rest: {dense_density} vs base {BASE_TREE_DENSITY}"
        );
    }

    #[test]
    fn every_tree_is_registered_in_the_static_grid_as_a_cylinder() {
        let trees = place_forest(FOREST_SEED);
        let ground = build_physics_heightfield();
        let mut world = World::new();
        let standing = populate_world(&mut world, &ground, &trees);
        let mut grid = StaticGrid::new(4.0);
        grid.rebuild(&world);

        assert_eq!(standing.len(), trees.len());
        assert_eq!(grid.len(), trees.len());
        for shape in grid.shapes() {
            match shape.blocker {
                Blocker::Cylinder { radius, top } => {
                    assert_eq!(radius, TREE_RADIUS);
                    assert!(top > 0.0);
                }
                other => panic!("a tree should be a cylinder blocker, got {other:?}"),
            }
        }
    }

    // --- the game, driven a frame at a time ------------------------------

    /// A frame rate to drive a headless test at.
    const FRAME: f32 = 1.0 / 60.0;

    /// The valley, started, with an engine to drive it through.
    fn scene(director: Director) -> (Valley, Engine) {
        let mut engine = Engine::new(8, 8);
        let mut valley = Valley::with_director(director);
        valley.start(&mut engine).expect("start cannot fail");
        (valley, engine)
    }

    /// Everything the loop does for one frame except drawing.
    fn frame(valley: &mut Valley, engine: &mut Engine, dt: f32) {
        engine.time.advance(dt);
        valley.update(engine);
        let mut steps = 0;
        while steps < 8 && engine.time.next_fixed_step() {
            valley.fixed_update(engine);
            steps += 1;
        }
        valley.track_head(engine);
    }

    /// Drive `seconds` of frames, feeding every one the same intent.
    fn hold(valley: &mut Valley, engine: &mut Engine, intent: Intent, seconds: f32) {
        let frames = (seconds / FRAME).round() as usize;
        for _ in 0..frames {
            valley.director = Director::Recorded(Script::new(vec![beat(FRAME * 2.0, intent)]));
            frame(valley, engine, FRAME);
        }
    }

    fn player_body(valley: &Valley, engine: &Engine) -> Body {
        valley.body(&engine.world, valley.player())
    }

    #[test]
    fn the_player_walks_where_the_head_is_looking_and_never_asks_for_an_angle() {
        let (mut valley, mut engine) = scene(Director::Live);
        let start = player_body(&valley, &engine).position;
        let ahead = valley.head.ahead();

        hold(&mut valley, &mut engine, walking(0.0), 1.0);
        let walked = player_body(&valley, &engine).position - start;
        let along = walked.x * ahead.x + walked.z * ahead.z;
        let across = walked.x * ahead.z - walked.z * ahead.x;
        assert!(along > 1.0, "{walked:?} along {ahead:?}");
        assert!(
            across.abs() < 0.5,
            "and roughly straight: {across} m sideways"
        );

        // What physics was handed is a vector, not a heading.
        let pending = engine
            .world
            .get::<PendingInput>(valley.player())
            .expect("the player has one");
        assert!(
            (pending.direction - ahead).length() < 1e-4,
            "{:?} against {ahead:?}",
            pending.direction
        );
        assert_eq!(pending.direction.y, 0.0, "a direction is horizontal");
    }

    #[test]
    fn turning_the_head_turns_the_walk_with_it() {
        let (mut valley, mut engine) = scene(Director::Live);
        let start = player_body(&valley, &engine).position;
        // A quarter turn to the right: what was to the right is now ahead.
        let was_right = valley.head.right();
        let quarter = std::f32::consts::FRAC_PI_2 / camera::MOUSE_SENSITIVITY;
        valley.head.turn(Vec2::new(quarter, 0.0));
        assert!((valley.head.ahead() - was_right).length() < 1e-4);

        hold(&mut valley, &mut engine, walking(0.0), 1.0);
        let walked = player_body(&valley, &engine).position - start;
        let along = walked.x * was_right.x + walked.z * was_right.z;
        assert!(along > 1.0, "{walked:?} along {was_right:?}");
    }

    #[test]
    fn a_run_is_twice_a_walk_while_there_is_breath_for_it() {
        let (mut valley, mut engine) = scene(Director::Live);
        let before = player_body(&valley, &engine).position;
        hold(&mut valley, &mut engine, walking(0.0), 1.0);
        let walked = (player_body(&valley, &engine).position - before).length();

        let (mut valley, mut engine) = scene(Director::Live);
        let before = player_body(&valley, &engine).position;
        let run = Intent {
            sprint: true,
            ..walking(0.0)
        };
        hold(&mut valley, &mut engine, run, 1.0);
        let ran = (player_body(&valley, &engine).position - before).length();
        assert!(
            ran > walked * 1.7,
            "{ran} m running against {walked} walking"
        );
    }

    #[test]
    fn a_jump_pressed_between_two_ticks_still_happens() {
        // The race `PendingInput` exists for: at 20 Hz under 60 Hz frames, two
        // frames in three run no tick at all. Press on one of those.
        let (mut valley, mut engine) = scene(Director::Live);
        // Land on a frame that will not tick: advance to just after a tick.
        frame(&mut valley, &mut engine, FRAME);
        let before = player_body(&valley, &engine).position.y;

        let jump = Intent {
            jump: true,
            ..Intent::default()
        };
        valley.director = Director::Recorded(Script::new(vec![beat(FRAME * 0.5, jump)]));
        engine.time.advance(FRAME);
        valley.update(&mut engine);
        assert!(
            !engine.time.next_fixed_step(),
            "this frame was supposed to run no tick"
        );
        assert!(
            engine
                .world
                .get::<PendingInput>(valley.player())
                .expect("the player has one")
                .jump_requested,
            "the press has to wait in the latch"
        );

        hold(&mut valley, &mut engine, Intent::default(), 0.3);
        let body = player_body(&valley, &engine);
        assert!(
            body.position.y > before + 0.2,
            "the jump was lost between frames: {} from {before}",
            body.position.y
        );
    }

    #[test]
    fn the_stamina_toy_shortens_a_run_and_a_jump_and_comes_back_when_standing() {
        let mut stamina = Stamina::full();
        let fresh = stamina.speed_scale(true);
        assert!(
            (fresh - 1.0).abs() < 1e-6,
            "a fresh sprint is a full sprint"
        );

        // Fifteen seconds of running is more than the scale holds.
        for _ in 0..900 {
            stamina.breathe(FRAME, true, true);
        }
        assert_eq!(stamina.0, 0.0);
        let spent = stamina.speed_scale(true);
        assert!(
            (spent - WALK_SPEED / RUN_SPEED).abs() < 1e-6,
            "an exhausted sprint is a walk, got {spent}"
        );
        assert!(
            stamina.jump_speed() < Stamina::full().jump_speed() * 0.8,
            "and the legs have less in them"
        );
        assert!(!stamina.can_jump(), "with nothing left, nothing leaves");

        // Standing gives it back; walking neither takes nor gives.
        for _ in 0..60 {
            stamina.breathe(FRAME, false, true);
        }
        assert_eq!(stamina.0, 0.0, "a walk is free but does not restore");
        for _ in 0..600 {
            stamina.breathe(FRAME, false, false);
        }
        assert!(stamina.speed_scale(true) > 0.95, "{}", stamina.0);
    }

    #[test]
    fn a_jump_costs_stamina_and_standing_pays_it_back() {
        let (mut valley, mut engine) = scene(Director::Live);
        let before = valley.stamina.0;
        let jump = Intent {
            jump: true,
            ..Intent::default()
        };
        valley.director = Director::Recorded(Script::new(vec![beat(FRAME * 2.0, jump)]));
        frame(&mut valley, &mut engine, FRAME);
        assert!(valley.stamina.0 < before, "a jump costs something");
    }

    // --- the tick-rate switch --------------------------------------------

    #[test]
    fn the_tick_rate_cycles_live_and_takes_physics_with_it() {
        let (mut valley, mut engine) = scene(Director::Live);
        assert!(
            (engine.time.fixed_delta - 1.0 / 20.0).abs() < 1e-7,
            "20 Hz by default"
        );

        let cycle = Intent {
            tick_rate: true,
            ..Intent::default()
        };
        let expected = [15.0, 60.0, 30.0, 20.0];
        for hz in expected {
            valley.director = Director::Recorded(Script::new(vec![beat(FRAME * 2.0, cycle)]));
            frame(&mut valley, &mut engine, FRAME);
            assert!((valley.tick_hz() - hz).abs() < 1e-3, "{hz}");
            assert!(
                (engine.time.fixed_delta - 1.0 / hz).abs() < 1e-7,
                "the clock has to move too"
            );
            assert!(
                (valley.physics.fixed_delta() - 1.0 / hz).abs() < 1e-7,
                "and so does physics, or the world runs fast"
            );
        }
    }

    // --- what a person at the keyboard can see and reach -----------------

    #[test]
    fn the_panel_names_the_rate_the_switch_is_on_and_the_breath_that_is_left() {
        // The switch is only usable live if the rate it lands on is legible
        // without reading the title bar of a window nobody screenshots.
        let (mut valley, mut engine) = scene(Director::Live);
        assert!(
            valley.hud_status(&engine).contains("20 Гц"),
            "{}",
            valley.hud_status(&engine)
        );
        valley.cycle_tick_rate(&mut engine);
        assert!(
            valley.hud_status(&engine).contains("15 Гц"),
            "{}",
            valley.hud_status(&engine)
        );

        assert!(valley.hud_status(&engine).contains("усталость: 100%"));
        let running = Intent {
            sprint: true,
            ..walking(0.0)
        };
        hold(&mut valley, &mut engine, running, 3.0);
        assert!(
            !valley.hud_status(&engine).contains("усталость: 100%"),
            "three seconds of running should show on the bar: {}",
            valley.hud_status(&engine)
        );
    }

    /// The opening frame of a live valley, with the panels on or off.
    fn opening_frame(hud: bool) -> Vec<u32> {
        let mut engine = Engine::new(320, 180);
        let mut valley = Valley::live();
        valley.start(&mut engine).expect("start cannot fail");
        valley.hud = hud;
        engine.framebuffer.clear(engine.clear_color);
        valley.render(&mut engine);
        engine.framebuffer.pixels().to_vec()
    }

    #[test]
    fn the_hud_is_drawn_over_the_scene_and_f_takes_it_away() {
        // Everything this card added is behind a key, and the keys are only
        // findable if they are on the screen.
        let with_hud = opening_frame(true);
        let without = opening_frame(false);
        assert_ne!(
            with_hud, without,
            "the panels have to be pixels, not a promise"
        );
        let changed = with_hud
            .iter()
            .zip(&without)
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            changed > 500,
            "only {changed} pixels of 320x180 carry the panels"
        );
    }

    #[test]
    fn the_arrow_keys_turn_the_head_the_same_way_the_mouse_does() {
        // There is no pointer lock, so a turn that runs the cursor into the
        // edge of the window has to have somewhere else to go.
        let mut input = Input::new();
        input.handle(&Event::KeyDown(Key::Right));
        let turning = Intent::read(&input, 1.0);
        assert!(turning.look.x > 0.0, "{:?}", turning.look);

        let mut head = Head::new(Vec3::ZERO, 0.0);
        head.turn(turning.look);
        assert!(
            (head.yaw - ARROW_LOOK_SPEED.to_radians()).abs() < 1e-4,
            "a second of Right is {ARROW_LOOK_SPEED} degrees, got {}",
            head.yaw.to_degrees()
        );

        // And half the frame is half the turn: it is a rate, not a per-frame
        // nudge that would turn faster on a faster machine.
        let half = Intent::read(&input, 0.5);
        assert!((half.look.x * 2.0 - turning.look.x).abs() < 1e-3);
    }

    #[test]
    fn the_recording_is_long_enough_to_hold_the_whole_walkthrough() {
        // Sixty frames of 0.34 s have to cover the script, or the video stops
        // before the tree comes down.
        let covered = RECORD_FRAMES as f32 * RECORD_FRAME_DELTA;
        let duration = walkthrough().duration();
        assert!(
            covered >= duration - 1e-3,
            "{RECORD_FRAMES} frames cover {covered} s of a {duration} s walk"
        );
        // And the frame is not a whole number of ticks, so most frames are
        // interpolated rather than landing on one.
        let ticks = RECORD_FRAME_DELTA / TICK_RATES[DEFAULT_TICK_RATE];
        assert!(
            (ticks - ticks.round()).abs() > 0.05,
            "{ticks} ticks a frame lands on a tick boundary every time"
        );
    }

    #[test]
    fn a_recorded_frame_fits_in_one_frame_of_catching_up() {
        // Two ceilings, and neither one of them says a word when it is
        // crossed: the loop runs at most `max_fixed_steps` ticks of catch-up
        // per frame, and the clock refuses to admit a frame longer than
        // `max_delta`. Go over either and the simulation falls behind a script
        // that is still being played at full speed, so the walk stops short of
        // the end of its own route — which is a thing you find out by looking
        // at sixty pictures, unless it is asserted here.
        let ticks = RECORD_FRAME_DELTA / TICK_RATES[DEFAULT_TICK_RATE];
        let budget = RunOptions::default().max_fixed_steps as f32;
        assert!(
            ticks <= budget,
            "a {RECORD_FRAME_DELTA} s frame is {ticks} ticks and the loop runs {budget}"
        );
        let (_, engine) = scene(Director::Live);
        assert!(
            RECORD_FRAME_DELTA < engine.time.max_delta,
            "the clock would clamp a {RECORD_FRAME_DELTA} s frame to {}",
            engine.time.max_delta
        );
    }

    #[test]
    fn the_world_runs_at_the_same_speed_whatever_the_tick_rate() {
        // Two seconds of walking at 60 Hz and at 15 Hz end up in the same
        // place, to within the distance one tick covers.
        let mut travelled = Vec::new();
        for rate in [0, 3] {
            let (mut valley, mut engine) = scene(Director::Live);
            while valley.tick_rate != rate {
                valley.cycle_tick_rate(&mut engine);
            }
            let start = player_body(&valley, &engine).position;
            hold(&mut valley, &mut engine, walking(0.0), 2.0);
            travelled.push((player_body(&valley, &engine).position - start).length());
        }
        assert!(
            (travelled[0] - travelled[1]).abs() < 0.4,
            "60 Hz walked {} m and 15 Hz walked {} m",
            travelled[0],
            travelled[1]
        );
    }

    // --- settlers ---------------------------------------------------------

    #[test]
    fn twelve_settlers_are_told_apart_by_height_and_shirt() {
        let (valley, _engine) = scene(Director::Live);
        assert_eq!(valley.settlers.len(), SETTLER_COUNT);
        for settler in &valley.settlers {
            assert!(
                (1.59..=1.91).contains(&settler.height),
                "{} m is not a person",
                settler.height
            );
        }
        for i in 0..SETTLER_COUNT {
            for j in (i + 1)..SETTLER_COUNT {
                let (a, b) = (&valley.settlers[i], &valley.settlers[j]);
                let colour = (a.shirt.r - b.shirt.r)
                    .abs()
                    .max((a.shirt.g - b.shirt.g).abs())
                    .max((a.shirt.b - b.shirt.b).abs());
                let height = (a.height - b.height).abs();
                assert!(
                    colour > 0.05 || height > 0.1,
                    "settlers {i} and {j} look the same: colour {colour}, height {height}"
                );
                assert!(height > 0.0, "and no two are exactly the same size");
            }
        }
    }

    #[test]
    fn the_settlers_wander_and_nobody_walks_through_anybody() {
        let (mut valley, mut engine) = scene(Director::Live);
        let before: Vec<Vec3> = valley
            .settlers
            .iter()
            .map(|s| valley.body(&engine.world, s.entity).position)
            .collect();

        for _ in 0..(6.0 / FRAME) as usize {
            frame(&mut valley, &mut engine, FRAME);
            let bodies: Vec<Body> = valley
                .settlers
                .iter()
                .map(|s| valley.body(&engine.world, s.entity))
                .chain(std::iter::once(player_body(&valley, &engine)))
                .collect();
            for i in 0..bodies.len() {
                for j in (i + 1)..bodies.len() {
                    let (a, b) = (bodies[i], bodies[j]);
                    let apart = Vec2::new(a.position.x - b.position.x, a.position.z - b.position.z);
                    assert!(
                        apart.length() > a.radius + b.radius - 1e-3,
                        "{i} and {j} are inside each other: {} m apart",
                        apart.length()
                    );
                }
            }
        }

        let moved = valley
            .settlers
            .iter()
            .zip(&before)
            .filter(|(s, start)| {
                (valley.body(&engine.world, s.entity).position - **start).length() > 1.0
            })
            .count();
        assert!(moved >= SETTLER_COUNT - 2, "only {moved} of them wandered");
    }

    #[test]
    fn a_settler_walking_into_the_player_slides_past_instead_of_through() {
        let (mut valley, mut engine) = scene(Director::Live);
        // Aim one settler straight at the player and take its patience away,
        // so it keeps pushing for the whole test.
        let target = Vec2::new(CAMPFIRE.x, CAMPFIRE.y);
        for _ in 0..(4.0 / FRAME) as usize {
            valley.settlers[0].target = target;
            valley.settlers[0].patience = WANDER_PATIENCE;
            valley.settlers[0].home = target;
            frame(&mut valley, &mut engine, FRAME);
            let settler = valley.body(&engine.world, valley.settlers[0].entity);
            let player = player_body(&valley, &engine);
            let apart = Vec2::new(
                settler.position.x - player.position.x,
                settler.position.z - player.position.z,
            );
            assert!(
                apart.length() > settler.radius + player.radius - 1e-3,
                "walked into the player: {} m",
                apart.length()
            );
        }
    }

    // --- the axe ----------------------------------------------------------

    /// Put the player `distance` metres from `target`, facing it. Walking
    /// there is the walkthrough's job; a test about the axe should not also be
    /// a test about pathing round a rock.
    fn stand_beside(valley: &mut Valley, engine: &mut Engine, target: Vec2, distance: f32) {
        let from = target - Vec2::new(distance, 0.0);
        let ground = valley.physics.ground.height_at(from.x, from.y);
        let player = valley.player();
        let body = engine
            .world
            .get_mut::<Body>(player)
            .expect("the player has a body");
        body.position = Vec3::new(from.x, ground, from.y);
        body.prev_position = body.position;
        valley.head.yaw = std::f32::consts::FRAC_PI_2;
    }

    /// Walk the player up to `target` and stop, by pointing the head at it.
    fn walk_to(valley: &mut Valley, engine: &mut Engine, target: Vec2, seconds: f32) {
        let frames = (seconds / FRAME).round() as usize;
        for _ in 0..frames {
            let body = player_body(valley, engine);
            let delta = target - Vec2::new(body.position.x, body.position.z);
            if delta.length() < 1.5 {
                break;
            }
            // Aim the head straight at the target, then walk forward.
            valley.head.yaw = delta.x.atan2(-delta.y);
            valley.director =
                Director::Recorded(Script::new(vec![beat(FRAME * 2.0, walking(0.0))]));
            frame(valley, engine, FRAME);
        }
    }

    #[test]
    fn a_felled_tree_comes_down_in_an_arc_and_lies_there_as_a_log() {
        let (mut valley, mut engine) = scene(Director::Live);
        stand_beside(&mut valley, &mut engine, LONE_TREE, 2.0);

        let tree = valley
            .swing_the_axe(&mut engine)
            .expect("the lone tree is within reach");
        assert!(
            engine
                .world
                .get::<FallingTree>(tree)
                .expect("it fell")
                .falling,
            "the axe starts the fall"
        );

        // A fall is a second and a half; the arc only ever goes one way.
        let mut reach = 0.0;
        for _ in 0..(2.5 / FRAME) as usize {
            frame(&mut valley, &mut engine, FRAME);
            let state = *engine.world.get::<FallingTree>(tree).expect("still there");
            assert!(state.reach() >= reach - 1e-4, "the trunk swung back");
            reach = state.reach();
        }
        let state = *engine.world.get::<FallingTree>(tree).expect("still there");
        assert!(state.fallen && !state.falling, "it should be down by now");

        // The standing blocker is gone and a log lies where the tip landed.
        assert!(
            engine.world.get::<Blocker>(tree).is_none(),
            "a felled tree is not still standing in the way"
        );
        let stump = valley
            .standing
            .iter()
            .find(|s| s.entity == tree)
            .expect("known tree")
            .position;
        let log_centre = stump + state.direction * (state.height * 0.5);
        let mut nearby = Vec::new();
        valley
            .physics
            .statics()
            .query(log_centre.x, log_centre.y, &mut nearby);
        let log = nearby
            .iter()
            .map(|&i| valley.physics.statics().shape(i))
            .find(|shape| matches!(shape.blocker, Blocker::Box { top, .. } if top == LOG_TOP));
        assert!(log.is_some(), "no log where the trunk came to rest");
    }

    #[test]
    fn a_log_is_stepped_over_and_the_eye_rises_to_say_so() {
        // The old log across the way up the valley: 0.3 m, under the step
        // height, so it is walked over rather than stopped at — and the head
        // has to show it, since the feet do it inside a single tick.
        let (mut valley, mut engine) = scene(Director::Live);
        walk_to(&mut valley, &mut engine, OLD_LOG, 30.0);
        // The log lies across the route; the way over it is up the route.
        valley.head.yaw = route_yaw();

        let before = Vec2::new(
            player_body(&valley, &engine).position.x,
            player_body(&valley, &engine).position.z,
        )
        .dot(ROUTE);
        let mut highest: f32 = f32::MIN;
        let mut on_the_log = false;
        for _ in 0..(4.0 / FRAME) as usize {
            hold(&mut valley, &mut engine, walking(0.0), FRAME);
            let body = player_body(&valley, &engine);
            let ground = valley
                .physics
                .ground
                .height_at(body.position.x, body.position.z);
            if body.position.y > ground + 0.2 {
                on_the_log = true;
                highest = highest.max(valley.head.eye().y - ground);
            }
        }
        let after = Vec2::new(
            player_body(&valley, &engine).position.x,
            player_body(&valley, &engine).position.z,
        )
        .dot(ROUTE);
        assert!(on_the_log, "the player never got onto the log");
        assert!(
            after > before + 1.0,
            "and never got over it: {before} to {after}"
        );
        assert!(
            highest > camera::EYE_HEIGHT + 0.15,
            "the eye barely noticed the log: {highest} m over the ground"
        );
    }

    // --- building ---------------------------------------------------------

    #[test]
    fn a_new_wall_blocks_at_once_and_walks_its_builder_out_over_half_a_second() {
        let (mut valley, mut engine) = scene(Director::Live);
        let start = player_body(&valley, &engine).position;
        valley.build_a_wall(&mut engine);

        // The blocker exists the moment it is built.
        let wall_facing = valley.walls.last().expect("a wall went up").facing;
        assert_eq!(valley.walls.last().expect("a wall went up").top, WALL_TOP);
        assert_eq!(valley.evictions.len(), 1, "the builder is standing in it");
        let exit = valley.evictions[0].to;

        let mut previous = Vec2::new(start.x, start.z);
        let mut worst_step: f32 = 0.0;
        let mut samples = 0;
        for _ in 0..(EVICTION_TIME / FRAME).round() as usize {
            frame(&mut valley, &mut engine, FRAME);
            let body = player_body(&valley, &engine);
            let here = Vec2::new(body.position.x, body.position.z);
            worst_step = worst_step.max((here - previous).length());
            previous = here;
            samples += 1;
        }
        assert!(samples > 0);
        assert!(
            worst_step < 0.1,
            "the builder was teleported out, {worst_step} m in one frame"
        );
        assert!(
            (previous - exit).length() < 0.05,
            "the builder should end up outside: {previous:?} against {exit:?}"
        );

        // And once outside, the wall keeps them outside.
        hold(&mut valley, &mut engine, Intent::default(), 0.5);
        let body = player_body(&valley, &engine);
        let point = Vec2::new(body.position.x, body.position.z);
        let shape = StaticShape {
            entity: valley.player(),
            base: start,
            blocker: Blocker::Box {
                half: WALL_HALF,
                facing: wall_facing,
                top: WALL_TOP,
            },
        };
        assert!(
            shape.resolve(point, body.radius).is_none(),
            "back inside the wall at {point:?}"
        );
    }

    // --- the recorded walkthrough ----------------------------------------

    /// Play the whole walkthrough at `dt` a frame, checking as it goes that
    /// the ground holds, nothing sticks and the camera stays on the body.
    ///
    /// Run at two frame rates on purpose. A window draws frames far shorter
    /// than a tick and the recording draws frames far longer than one, and the
    /// script is played by the frame while the world is stepped by the tick:
    /// the two rates are the two ends of that, and the coarse one is the one
    /// the card's video is actually made at.
    fn walk_the_script(dt: f32) -> (Valley, Engine) {
        let script = walkthrough();
        let duration = script.duration();
        let (mut valley, mut engine) = scene(Director::Recorded(script));

        let mut previous_eye = valley.head.eye();
        let mut stuck_for = 0;
        let mut worst_stuck = 0;
        let mut travelled = 0.0;
        let mut airborne = false;
        let mut landed = 0;
        let mut previous = player_body(&valley, &engine).position;

        let frames = (duration / dt).round() as usize;
        for step in 0..frames {
            frame(&mut valley, &mut engine, dt);
            let body = player_body(&valley, &engine);
            let ground = valley
                .physics
                .ground
                .height_at(body.position.x, body.position.z);
            assert!(
                body.position.y >= ground - 1e-3,
                "frame {step}: a foot went under the ground at {:?}",
                body.position
            );

            let moved = (body.position - previous).length();
            travelled += moved;
            // Standing still is fine when the script asks for it; being stuck
            // is standing still while asking to move.
            let asking = engine
                .world
                .get::<PendingInput>(valley.player())
                .expect("the player has one")
                .direction
                .length()
                > 0.5;
            if asking && moved < 0.002 {
                stuck_for += 1;
                worst_stuck = worst_stuck.max(stuck_for);
            } else {
                stuck_for = 0;
            }
            previous = body.position;

            if !body.grounded {
                airborne = true;
            } else if airborne {
                airborne = false;
                landed += 1;
            }

            // The head never parts company with the body: the whole of the
            // camera's own motion is centimetres, on top of the feet.
            let eye = valley.head.eye();
            let offset =
                eye.y - (body.render_position(engine.time.fixed_alpha()).y + camera::EYE_HEIGHT);
            assert!(
                offset.abs() < 0.35,
                "{dt} s frames, frame {step}: the eye is {offset} m off the body"
            );
            // A frame this long moves the body a long way by itself; what
            // must not happen is the eye moving further than the feet did.
            let budget = RUN_SPEED * dt + 0.1;
            assert!(
                (eye - previous_eye).length() < budget,
                "{dt} s frames, frame {step}: the camera jumped {} m",
                (eye - previous_eye).length()
            );
            previous_eye = eye;
        }

        assert!(
            travelled > 25.0,
            "{dt} s frames: the walkthrough only covered {travelled} m"
        );
        assert!(
            (worst_stuck as f32) * dt < 1.0,
            "{dt} s frames: the player was stuck for {worst_stuck} frames while asking to move"
        );
        assert!(
            landed >= 2,
            "{dt} s frames: the run should leave the ground and come back"
        );

        // And it ends with a tree down.
        let felled = valley
            .standing
            .iter()
            .filter(|tree| {
                engine
                    .world
                    .get::<FallingTree>(tree.entity)
                    .is_some_and(|state| state.fallen)
            })
            .count();
        assert_eq!(
            felled, 1,
            "{dt} s frames: the walkthrough fells exactly one tree"
        );
        (valley, engine)
    }

    #[test]
    fn the_walkthrough_gets_through_the_whole_valley_without_falling_through_it() {
        walk_the_script(FRAME);
    }

    #[test]
    fn the_walkthrough_ends_in_the_same_place_at_the_rate_it_is_recorded_at() {
        // The recording draws frames seven and a half ticks long. If the walk
        // came out somewhere else at that rate — because a frame asked for
        // more catch-up than the loop will run, say — the sixty pictures would
        // be of a different walk than every test here drives.
        let (fine, fine_engine) = walk_the_script(FRAME);
        let (coarse, coarse_engine) = walk_the_script(RECORD_FRAME_DELTA);
        let a = player_body(&fine, &fine_engine).position;
        let b = player_body(&coarse, &coarse_engine).position;
        assert!(
            (a - b).length() < 2.0,
            "the fine run ended at {a:?} and the recorded one at {b:?}"
        );
        // And what there is to see at the end is the felled trunk, close
        // enough and in front: the last shot of the video is of a log.
        let log = coarse
            .standing
            .iter()
            .find(|tree| tree.position == LONE_TREE)
            .expect("the lone tree is planted");
        let eye = coarse.head.eye();
        let to_log = Vec2::new(log.position.x - eye.x, log.position.y - eye.z);
        let ahead = coarse.head.ahead();
        assert!(
            to_log.length() < AXE_REACH,
            "the log ended up {} m away",
            to_log.length()
        );
        assert!(
            to_log.normalized().dot(Vec2::new(ahead.x, ahead.z)) > 0.8,
            "the log is not in front of the camera"
        );
        assert!(
            coarse.head.pitch < -0.2,
            "the head has to be tipped down to see a 0.3 m log at two metres, \
             and it is at {} rad",
            coarse.head.pitch
        );
    }

    #[test]
    fn a_script_fires_an_edge_once_and_holds_a_level() {
        let jump = Intent {
            jump: true,
            walk: Vec2::new(0.0, 1.0),
            ..Intent::default()
        };
        let mut script = Script::new(vec![beat(0.1, jump), beat(0.1, Intent::default())]);
        let first = script.frame(0.02);
        assert!(first.jump, "the edge fires on the first frame of its beat");
        for _ in 0..4 {
            let held = script.frame(0.02);
            assert!(!held.jump, "and not again while the beat is held");
            assert_eq!(held.walk, jump.walk, "but the level holds");
        }
        script.frame(0.02);
        assert_eq!(script.frame(0.02).walk, Vec2::ZERO, "on to the next beat");
    }

    #[test]
    fn a_frame_longer_than_a_beat_still_fires_that_beat_s_edges() {
        // The recording's frames are seven and a half ticks long and swallow
        // whole beats. An axe swing dropped on the floor there is a swing that
        // happens in every test and in nobody's video.
        let chop = Intent {
            chop: true,
            ..Intent::default()
        };
        let build = Intent {
            build: true,
            ..Intent::default()
        };
        let mut script = Script::new(vec![
            beat(0.1, Intent::default()),
            beat(0.1, chop),
            beat(0.1, build),
            beat(0.1, Intent::default()),
        ]);
        let swallowed = script.frame(0.35);
        assert!(swallowed.chop, "the swing was inside the frame");
        assert!(swallowed.build, "and so was the wall");
        assert!(
            !script.frame(0.1).chop,
            "and neither of them fires a second time"
        );
    }

    #[test]
    fn a_frame_that_spans_two_beats_turns_at_each_beat_s_own_rate() {
        // Charging the whole frame to the beat it started in is how a route
        // written in degrees ends up pointing somewhere else entirely.
        let mut coarse = Script::new(vec![beat(0.2, turning(90.0)), beat(0.2, turning(-90.0))]);
        let turned = coarse.frame(0.4).look.x;
        assert!(
            turned.abs() < 1e-3,
            "ninety degrees one way and ninety the other is nowhere, not {turned} px"
        );
    }

    #[test]
    fn a_script_turns_the_head_by_the_same_angle_at_any_frame_rate() {
        // `look` is a rate, not a delta, or the recording would turn a
        // different amount than the window does.
        let turning = walking(90.0);
        let mut total = Vec::new();
        for dt in [1.0 / 60.0, 0.24] {
            let mut script = Script::new(vec![beat(1.2, turning)]);
            let mut turned = 0.0;
            let frames = (1.2f32 / dt).round() as usize;
            for _ in 0..frames {
                turned += script.frame(dt).look.x;
            }
            total.push(turned);
        }
        assert!(
            (total[0] - total[1]).abs() < 2.0,
            "{} pixels against {}",
            total[0],
            total[1]
        );
    }
}
