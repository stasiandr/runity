//! Ropes, cables and chains: a line's `rope`. `rope: (to: (6.0, 0.0,
//! 0.0), slack: 0.08)` strings one from the entity to a point in its space
//! — a washing line, a rope bridge's rail, a power line, a lamp's chain —
//! sagging by its slack, swinging in the scene's wind, lying on what it
//! meets. Tied to another entity (`end: EntityRef(…)`), it follows both.
//! With `ends: Start` it hangs from the entity alone.
//!
//! Each is a Cosserat rod ([`crate::rod`]): a rope bends at a touch, a
//! cable holds its line and comes straight out of where it is clamped, a
//! chain does not stretch and is drawn as links, every other one turned a
//! quarter, as they hang through each other.
//!
//! Only a look by default: every machine swings its own, with nothing sent
//! (DNA, postulate 4). Its steps are fixed, so the same wind swings it the
//! same way.

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use runity_core::wind::Wind;
use runity_core::world::WorldTransform;
use runity_core::EntityRef;
use runity_geometry::mesh_asset::Vertex;

use crate::obstacle::{Obstacle, Obstacles};
use crate::rod::{Give, Rod};

/// A rope, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rope {
    /// Where its far end is tied, in the entity's space — or, with `end`,
    /// in that entity's.
    pub to: Vec3,
    /// Tied at its far end to this entity instead, at `to` in its space.
    #[serde(skip_serializing_if = "unlinked")]
    pub end: EntityRef,
    /// Which ends are tied.
    pub ends: Ends,
    /// What it is: how it bends and how it is drawn.
    pub kind: RopeKind,
    /// How much longer than the straight line it is: 0 taut, 0.1 a good
    /// sag. With `ends: Start`, its length is `to`'s.
    pub slack: f32,
    /// Links along it.
    pub segments: u32,
    /// How thick, metres: a tube so wide, or links so big.
    pub thickness: f32,
    /// How much it holds its line, from 0 (limp) to 1 (a stiff cable);
    /// its kind's when left out.
    #[serde(skip_serializing_if = "Option::is_none", with = "runity_core::defaults::plain")]
    pub stiffness: Option<f32>,
    /// How much the wind takes it: a thin cord more, a hawser less.
    pub catch: f32,
}

fn unlinked(end: &EntityRef) -> bool {
    end.0.is_none()
}

/// Which ends of a rope are tied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Ends {
    /// Both: strung between.
    #[default]
    Both,
    /// Only where it starts: it hangs from the entity.
    Start,
}

/// What a rope is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RopeKind {
    /// Bends at a touch, stretches a little, drawn as a tube.
    #[default]
    Rope,
    /// Holds its line, comes straight out of its ends, drawn as a tube.
    Cable,
    /// Does not stretch, drawn as links.
    Chain,
}

impl Default for Rope {
    fn default() -> Self {
        Self {
            to: Vec3::new(4.0, 0.0, 0.0),
            end: EntityRef::default(),
            ends: Ends::Both,
            kind: RopeKind::Rope,
            slack: 0.08,
            segments: 24,
            thickness: 0.025,
            stiffness: None,
            catch: 0.4,
        }
    }
}

runity_core::impl_parts! {
    Rope => "rope";
}

/// The rope of a line, read off it.
pub trait RopeLine {
    fn rope(&self) -> Option<Rope>;
}

impl RopeLine for runity_core::EntityDesc {
    fn rope(&self) -> Option<Rope> {
        self.part()
    }
}

impl RopeLine for runity_core::scene::Override {
    fn rope(&self) -> Option<Rope> {
        self.part()
    }
}

/// Seconds a step: ropes run at their own fixed rate, whatever the frame.
pub const STEP: f32 = 1.0 / 60.0;
/// Substeps a step, one pass of the constraints each.
pub const SUBSTEPS: usize = 8;
/// Metres a second of wind at `strength: 1`, as the physics' wind.
const WIND_SPEED: f32 = 4.0;
/// Kilograms a metre: a rope's own weight, and its kind's multiple.
const WEIGHT: f32 = 0.2;

impl Rope {
    /// How much it holds its line, 0 to 1.
    pub fn stiffness(&self) -> f32 {
        self.stiffness
            .unwrap_or(match self.kind {
                RopeKind::Rope => 0.0,
                RopeKind::Cable => 0.9,
                RopeKind::Chain => 0.0,
            })
            .clamp(0.0, 1.0)
    }

    /// How it gives, as compliances.
    fn give(&self) -> Give {
        // Stiffness to compliance: 1 does not give at all, 0 gives so
        // much that bending is all but free.
        let s = self.stiffness();
        let bend = if s >= 1.0 { 0.0 } else { 2e-4 * (1.0 / s.max(1e-4) - 1.0) };
        Give {
            stretch: match self.kind {
                RopeKind::Rope => 1e-7,
                RopeKind::Cable | RopeKind::Chain => 0.0,
            },
            bend,
            twist: bend,
        }
    }

    fn weight(&self) -> f32 {
        match self.kind {
            RopeKind::Chain => WEIGHT * 10.0,
            RopeKind::Cable => WEIGHT * 2.0,
            RopeKind::Rope => WEIGHT,
        }
    }
}

/// A rope as it moves: the component [`run_ropes`] steps.
#[derive(Debug, Clone)]
pub struct RopeState {
    pub rope: Rope,
    /// The scene's wind, which swings it: whoever spawns the scene sets
    /// it ([`set_wind`]).
    pub wind: Wind,
    rod: Option<Rod>,
    /// The entity its far end is tied to, once found.
    end: Option<hecs::Entity>,
    /// Where its tied ends were at the last step, to move them smoothly
    /// through the substeps.
    ends_were: Option<(Mat4, Mat4)>,
    /// A cable's end links, turned from their entities as they were hung.
    clamps: (Quat, Quat),
    /// Each particle's pull this substep: kept to be written again.
    pull: Vec<Vec3>,
    /// The obstacles near it this step.
    near: Vec<Obstacle>,
    time: f32,
    owed: f32,
}

impl RopeState {
    pub fn new(rope: Rope) -> Self {
        Self {
            rope,
            wind: Wind::default(),
            rod: None,
            end: None,
            ends_were: None,
            clamps: (Quat::IDENTITY, Quat::IDENTITY),
            pull: Vec::new(),
            near: Vec::new(),
            time: 0.0,
            owed: 0.0,
        }
    }

    /// Its rod, once it has been hung.
    pub fn rod(&self) -> Option<&Rod> {
        self.rod.as_ref()
    }

    /// Where its particles are, in the world; none before it is hung.
    pub fn points(&self) -> &[Vec3] {
        self.rod.as_ref().map_or(&[], |r| &r.particles.x)
    }

    /// Hung between `start` and `end` as it would rest: sagging in a
    /// parabola as deep as its slack asks, or straight down from the start.
    /// A parabola through something solid would be pushed out the wrong
    /// side of it; such a rope starts along the straight line instead and
    /// falls onto what is under it.
    fn hang(&mut self, start: Mat4, end: Mat4, obstacles: &[Obstacle]) {
        let r = self.rope;
        let n = r.segments.clamp(2, 256) as usize + 1;
        let a = start.transform_point3(Vec3::ZERO);
        let (points, length) = match r.ends {
            Ends::Both => {
                let b = end.transform_point3(r.to);
                let chord = a.distance(b).max(1e-3);
                let length = chord * (1.0 + r.slack.max(0.0));
                // A parabola of length L over chord c sags by
                // √(3c(L − c)/8).
                let sag = (3.0 * chord * (length - chord) / 8.0).max(0.0).sqrt();
                let r = (r.thickness * 0.5).max(0.002);
                let through = (0..n).any(|i| {
                    let t = i as f32 / (n - 1) as f32;
                    let p = a.lerp(b, t) - Vec3::Y * (4.0 * sag * t * (1.0 - t));
                    obstacles.iter().any(|o| o.contact(p, r).is_some())
                });
                let sag = if through { 0.0 } else { sag };
                let points = (0..n)
                    .map(|i| {
                        let t = i as f32 / (n - 1) as f32;
                        a.lerp(b, t) - Vec3::Y * (4.0 * sag * t * (1.0 - t))
                    })
                    .collect();
                (points, length)
            }
            Ends::Start => {
                let length = start.transform_vector3(r.to).length().max(1e-3);
                let down = start.transform_vector3(r.to).normalize_or(-Vec3::Y);
                let points = (0..n).map(|i| a + down * (length * i as f32 / (n - 1) as f32)).collect();
                (points, length)
            }
        };
        let link = length / (n - 1) as f32;
        let mass = (r.weight() * link).max(1e-4);
        let mut rod = Rod::new(points, vec![link; n - 1], mass, r.give());
        rod.radius = (r.thickness * 0.5).max(0.002);
        rod.pin(0, true, mass);
        if r.ends == Ends::Both {
            rod.pin(n - 1, true, mass);
        }
        if r.kind == RopeKind::Cable {
            // Clamped: out of its ends as they point.
            rod.clamp(0, true, mass);
            if r.ends == Ends::Both {
                rod.clamp(n - 2, true, mass);
            }
            self.clamps = (clamped_from(&rod, 0, start), clamped_from(&rod, n - 2, end));
        }
        self.rod = Some(rod);
        self.ends_were = Some((start, end));
    }

    /// Along by `seconds`, its start at `start` and its far end in the
    /// space of `end` (the entity's own when it is tied to nothing else),
    /// in `wind`, lying on `obstacles`.
    pub fn advance(&mut self, start: Mat4, end: Mat4, wind: &Wind, obstacles: &[Obstacle], seconds: f32) {
        if self.rod.is_none() {
            self.hang(start, end, obstacles);
        }
        self.owed = (self.owed + seconds.max(0.0)).min(0.25);
        while self.owed >= STEP {
            self.owed -= STEP;
            self.step(start, end, wind, obstacles);
        }
    }

    fn step(&mut self, start: Mat4, end: Mat4, wind: &Wind, obstacles: &[Obstacle]) {
        let r = self.rope;
        let (start_was, end_was) = self.ends_were.unwrap_or((start, end));
        self.ends_were = Some((start, end));
        let Some(rod) = self.rod.as_mut() else { return };
        // Only what is near enough to touch this step: the rope's bounds,
        // with room for how far it can go in one.
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in rod.particles.x.iter().chain([&start.w_axis.truncate(), &end.transform_point3(r.to)]) {
            low = low.min(*p);
            high = high.max(*p);
        }
        let fastest = rod.particles.v.iter().map(|v| v.length()).fold(0.0, f32::max);
        let room = Vec3::splat(rod.radius + fastest * STEP + 0.25);
        self.near.clear();
        self.near.extend(obstacles.iter().filter(|o| o.near(low - room, high + room)).copied());
        let obstacles = &self.near;
        let h = STEP / SUBSTEPS as f32;
        let level = Vec3::new(wind.direction.x, 0.0, wind.direction.z).normalize_or_zero();
        let speed = WIND_SPEED * wind.strength.max(0.0);
        let catch = r.catch.max(0.0) / r.weight().max(1e-3) * WEIGHT;
        let last = rod.particles.len() - 1;
        let (a_was, a) = (start_was.transform_point3(Vec3::ZERO), start.transform_point3(Vec3::ZERO));
        let (b_was, b) = (end_was.transform_point3(r.to), end.transform_point3(r.to));
        let clamp_turns = r.kind == RopeKind::Cable;
        let (turn_a, turn_b) = (turn_of(start), turn_of(end));
        let (rest_a, rest_b) = self.clamps;
        for s in 0..SUBSTEPS {
            let t = (s + 1) as f32 / SUBSTEPS as f32;
            self.time += h;
            // The tied ends go where the entities are going, a share each
            // substep, so a swung post drags its rope smoothly.
            rod.particles.x[0] = a_was.lerp(a, t);
            if r.ends == Ends::Both {
                rod.particles.x[last] = b_was.lerp(b, t);
            }
            if clamp_turns {
                rod.set_turn(0, turn_a * rest_a);
                if r.ends == Ends::Both {
                    rod.set_turn(last - 1, turn_b * rest_b);
                }
            }
            let (x, v) = (&rod.particles.x, &rod.particles.v);
            self.pull.clear();
            self.pull.extend((0..=last).map(|i| {
                let p = x[i];
                let along = p.dot(level);
                let gust = 0.6 + 0.4 * (self.time * 1.1 - along * 0.3 + i as f32 * 0.05).sin();
                let air = level * speed * gust - v[i];
                // A cord takes the wind across itself, not along.
                let tangent = (x[(i + 1).min(last)] - x[i.saturating_sub(1)]).normalize_or_zero();
                let across = air - tangent * air.dot(tangent);
                Vec3::new(0.0, -9.81, 0.0) + across * catch
            }));
            let pull = &self.pull;
            rod.substep(h, |i| pull[i], obstacles);
        }
    }

    /// A tube along it through a smooth curve through its particles, in
    /// the space of `placed` (what it is drawn at): `around` sides, `per`
    /// rings a link. Its seam twists as the rope does.
    pub fn tube(&self, placed: Mat4, around: usize, per: usize) -> (Vec<Vertex>, Vec<u32>) {
        let Some(rod) = &self.rod else {
            return (Vec::new(), Vec::new());
        };
        let back = placed.inverse();
        let x = &rod.particles.x;
        let n = x.len();
        let (around, per) = (around.max(3), per.max(1));
        let radius = (self.rope.thickness * 0.5).max(0.001);
        let rings = (n - 1) * per + 1;
        let mut vertices = Vec::with_capacity(rings * (around + 1));
        let mut along = 0.0;
        let mut last = None;
        for ring in 0..rings {
            let (k, t) = ((ring / per).min(n - 2), (ring - (ring / per).min(n - 2) * per) as f32 / per as f32);
            let (k, t) = if ring == rings - 1 { (n - 2, 1.0) } else { (k, t) };
            let p = catmull_rom(x, k, t);
            let tangent = (catmull_rom(x, k, (t + 0.01).min(1.0)) - catmull_rom(x, k, (t - 0.01).max(0.0))).normalize_or(Vec3::X);
            let (q0, mut q1) = (rod.turn_at(k), rod.turn_at(k + 1));
            if q0.dot(q1) < 0.0 {
                q1 = -q1;
            }
            let turn = q0.slerp(q1, t);
            let mut side = turn * Vec3::X;
            side = (side - tangent * side.dot(tangent)).normalize_or(tangent.any_orthonormal_vector());
            let up = tangent.cross(side);
            if let Some(was) = last {
                along += p.distance(was);
            }
            last = Some(p);
            // The picture wraps once round, and along as far as it is
            // round, so a pattern keeps its shape.
            let v = along / (std::f32::consts::TAU * radius);
            let local = back.transform_point3(p);
            for j in 0..=around {
                let a = j as f32 / around as f32 * std::f32::consts::TAU;
                let normal = side * a.cos() + up * a.sin();
                let n = back.transform_vector3(normal).normalize_or(normal);
                vertices.push(Vertex {
                    position: (local + back.transform_vector3(normal * radius)).to_array(),
                    normal: n.to_array(),
                    uv: [j as f32 / around as f32, v],
                });
            }
        }
        let stride = (around + 1) as u32;
        let mut indices = Vec::with_capacity((rings - 1) * around * 6);
        for ring in 0..rings as u32 - 1 {
            for j in 0..around as u32 {
                let a = ring * stride + j;
                let (b, c, d) = (a + 1, a + stride, a + stride + 1);
                indices.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
        (vertices, indices)
    }

    /// Where each link of a chain is, in the world: at the middle of its
    /// link, along it, every other one turned a quarter, as big as its
    /// link — for `builtin:link`, a metre long.
    pub fn links(&self) -> Vec<Mat4> {
        let Some(rod) = &self.rod else {
            return Vec::new();
        };
        let x = &rod.particles.x;
        let quarter = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        (0..rod.links())
            .map(|k| {
                let middle = (x[k] + x[k + 1]) * 0.5;
                let along = (x[k + 1] - x[k]).normalize_or(rod.turn[k] * Vec3::Z);
                // Its turn, set along where the link actually lies.
                let turn = Quat::from_rotation_arc(rod.turn[k] * Vec3::Z, along) * rod.turn[k];
                let turn = if k % 2 == 1 { turn * quarter } else { turn };
                // The links reach into each other: a link's inside is a
                // little over half its length.
                let size = rod.rest[k] * 1.55;
                let wide = (self.rope.thickness / 0.16).max(size * 0.2);
                Mat4::from_scale_rotation_translation(Vec3::new(wide.min(size), wide.min(size), size), turn, middle)
            })
            .collect()
    }
}

/// The turn of a placed matrix, without its scale.
fn turn_of(placed: Mat4) -> Quat {
    placed.to_scale_rotation_translation().1
}

/// The turn of link `k` as it lies, relative to the turn of `placed`: what
/// it keeps as the entity turns.
fn clamped_from(rod: &Rod, k: usize, placed: Mat4) -> Quat {
    turn_of(placed).inverse() * rod.turn[k]
}

/// A point on the smooth curve through `x`, between `x[k]` and `x[k + 1]`.
fn catmull_rom(x: &[Vec3], k: usize, t: f32) -> Vec3 {
    let n = x.len();
    let p1 = x[k];
    let p2 = x[(k + 1).min(n - 1)];
    let p0 = if k == 0 { p1 * 2.0 - p2 } else { x[k - 1] };
    let p3 = if k + 2 >= n { p2 * 2.0 - p1 } else { x[k + 2] };
    let (t2, t3) = (t * t, t * t * t);
    0.5 * (2.0 * p1 + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
}

/// Every rope swung by this wind: the scene's, as it is spawned or
/// changes.
pub fn set_wind(world: &mut hecs::World, wind: Wind) {
    for state in world.query_mut::<&mut RopeState>() {
        state.wind = wind;
    }
}

/// Step every rope by `seconds` in its wind, lying on `obstacles`. Its far
/// end found where it is tied.
pub fn run_ropes(world: &mut hecs::World, seconds: f32, obstacles: &Obstacles) {
    let mut todo = Vec::new();
    for (entity, state, placed) in world.query::<(hecs::Entity, &RopeState, &WorldTransform)>().iter() {
        todo.push((entity, placed.0, state.rope.end, state.end));
    }
    let mut near = Vec::new();
    for (entity, start, link, found) in todo {
        let end = if link.0.is_some() {
            let known = found.filter(|e| world.contains(*e)).or_else(|| link.get(world));
            let Some(tied) = known else { continue };
            let Ok(placed) = world.get::<&WorldTransform>(tied).map(|p| p.0) else { continue };
            if let Ok(mut state) = world.get::<&mut RopeState>(entity) {
                state.end = Some(tied);
            }
            placed
        } else {
            start
        };
        if let Ok(mut state) = world.get::<&mut RopeState>(entity) {
            // What is near where it is and where it is tied: a rope not
            // yet hung may reach anywhere between its ends and its length
            // below them.
            let (a, b) = (start.w_axis.truncate(), end.transform_point3(state.rope.to));
            let reach = a.distance(b) * (1.0 + state.rope.slack.max(0.0)) + state.rope.to.length() + 1.0;
            let (mut low, mut high) = (a.min(b), a.max(b));
            for p in state.points() {
                low = low.min(*p);
                high = high.max(*p);
            }
            let room = if state.points().is_empty() { reach } else { 1.0 + seconds * 20.0 };
            obstacles.near(low - Vec3::splat(room), high + Vec3::splat(room), &mut near);
            let wind = state.wind;
            state.advance(start, end, &wind, &near, seconds);
        }
    }
}

/// The soft module's dresser for ropes ([`runity_core::world::Dress`]): a
/// line's `rope`, hung afresh when it changes.
pub struct RopeDress;

impl runity_core::world::Dress for RopeDress {
    fn parts(&self) -> &[&'static str] {
        &["rope"]
    }

    fn dress(
        &mut self,
        line: &runity_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: runity_core::world::Changed,
        _: &mut Vec<runity_core::world::Unresolved>,
    ) {
        match line.rope() {
            Some(rope) => {
                let _ = world.insert_one(entity, RopeState::new(rope));
            }
            None => {
                let _ = world.remove_one::<RopeState>(entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn still() -> Wind {
        Wind {
            direction: Vec3::X,
            strength: 0.0,
        }
    }

    fn settle(state: &mut RopeState, posts: Mat4, wind: Wind, obstacles: &[Obstacle]) {
        for _ in 0..150 {
            state.advance(posts, posts, &wind, obstacles, 1.0 / 30.0);
        }
    }

    #[test]
    fn a_slack_rope_sags_in_the_middle_as_long_as_it_is_and_swings_in_a_crosswind() {
        let line = Rope {
            to: Vec3::new(6.0, 0.0, 0.0),
            slack: 0.08,
            ..Rope::default()
        };
        let posts = Mat4::from_translation(Vec3::new(0.0, 3.0, 0.0));
        let mut hung = RopeState::new(line);
        settle(&mut hung, posts, still(), &[]);
        let p = hung.points();
        let middle = p[p.len() / 2];
        // A rope 8% longer than 6 m sags about 1.2 m in its middle.
        let sag = 3.0 - middle.y;
        assert!((0.9..1.5).contains(&sag), "sags {sag}");
        // Its ends stay tied.
        assert!(p[0].distance(Vec3::new(0.0, 3.0, 0.0)) < 1e-4);
        assert!(p[p.len() - 1].distance(Vec3::new(6.0, 3.0, 0.0)) < 1e-4);
        // And it is as long as it is, near enough.
        let length: f32 = p.windows(2).map(|w| w[0].distance(w[1])).sum();
        assert!((length - 6.48).abs() < 0.05, "{length}");

        let mut blown = RopeState::new(line);
        settle(&mut blown, posts, Wind { direction: Vec3::Z, strength: 3.0 }, &[]);
        let swung = blown.points()[blown.points().len() / 2];
        assert!(swung.z > 0.15, "swings downwind: {swung}");
    }

    #[test]
    fn a_rope_too_long_to_hang_clear_lies_on_the_ground() {
        let line = Rope {
            to: Vec3::new(4.0, 0.0, 0.0),
            slack: 1.0,
            ..Rope::default()
        };
        let posts = Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0));
        let mut state = RopeState::new(line);
        settle(&mut state, posts, still(), &[Obstacle::ground(0.0)]);
        let lowest = state.points().iter().map(|p| p.y).fold(f32::MAX, f32::min);
        let r = line.thickness * 0.5;
        assert!(lowest > r - 0.01, "above the ground: {lowest}");
        let lying = state.points().iter().filter(|p| p.y < r + 0.02).count();
        assert!(lying > 4, "its middle lies along the ground: {lying} points");
    }

    #[test]
    fn a_chain_hangs_from_its_start_without_stretching_and_its_links_alternate() {
        let line = Rope {
            to: Vec3::new(0.0, -2.0, 0.0),
            ends: Ends::Start,
            kind: RopeKind::Chain,
            segments: 16,
            thickness: 0.03,
            ..Rope::default()
        };
        let hook = Mat4::from_translation(Vec3::new(0.0, 4.0, 0.0));
        let mut state = RopeState::new(line);
        // Swung sideways, then let hang.
        settle(&mut state, hook, Wind { direction: Vec3::X, strength: 2.0 }, &[]);
        settle(&mut state, hook, still(), &[]);
        let p = state.points();
        let length: f32 = p.windows(2).map(|w| w[0].distance(w[1])).sum();
        assert!((length - 2.0).abs() < 0.01, "a chain does not stretch: {length}");
        let bottom = *p.last().unwrap();
        assert!(bottom.distance(Vec3::new(0.0, 2.0, 0.0)) < 0.1, "hangs straight: {bottom}");
        let links = state.links();
        assert_eq!(links.len(), 16);
        // Neighbouring links lie across each other.
        let plane = |m: &Mat4| m.transform_vector3(Vec3::Y).normalize();
        assert!(plane(&links[4]).dot(plane(&links[5])).abs() < 0.2);
    }

    #[test]
    fn a_cable_comes_straight_out_of_its_clamp_where_a_rope_drops() {
        // Hung from a wall, pointing out level, 1.5 m long.
        let out = |kind: RopeKind| {
            let line = Rope {
                to: Vec3::new(1.5, 0.0, 0.0),
                ends: Ends::Start,
                kind,
                segments: 12,
                ..Rope::default()
            };
            let wall = Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0));
            let mut state = RopeState::new(line);
            settle(&mut state, wall, still(), &[]);
            let p = state.points();
            (p[1] - p[0]).normalize()
        };
        assert!(out(RopeKind::Cable).x > 0.9, "{}", out(RopeKind::Cable));
        assert!(out(RopeKind::Rope).y < -0.9, "{}", out(RopeKind::Rope));
    }

    #[test]
    fn a_rope_tied_to_another_entity_follows_it_and_the_tube_is_whole() {
        let mut world = hecs::World::new();
        let id: runity_core::EntityId = "00000000000000aa".parse().unwrap();
        let post = world.spawn((
            WorldTransform(Mat4::from_translation(Vec3::new(5.0, 2.0, 0.0))),
            runity_core::world::SceneId(id),
        ));
        let rope = world.spawn((
            WorldTransform(Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0))),
            RopeState::new(Rope {
                to: Vec3::ZERO,
                end: EntityRef::to(id),
                slack: 0.02,
                ..Rope::default()
            }),
        ));
        set_wind(&mut world, still());
        for _ in 0..30 {
            run_ropes(&mut world, 1.0 / 30.0, &Obstacles::default());
        }
        let state = world.get::<&RopeState>(rope).unwrap();
        assert!(state.points().last().unwrap().distance(Vec3::new(5.0, 2.0, 0.0)) < 1e-4);
        drop(state);
        // The post moves: the rope's end goes with it.
        *world.get::<&mut WorldTransform>(post).unwrap() = WorldTransform(Mat4::from_translation(Vec3::new(5.0, 3.0, 1.0)));
        run_ropes(&mut world, 1.0 / 30.0, &Obstacles::default());
        let state = world.get::<&RopeState>(rope).unwrap();
        assert!(state.points().last().unwrap().distance(Vec3::new(5.0, 3.0, 1.0)) < 1e-4);
        let (vertices, indices) = state.tube(Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)), 6, 3);
        assert_eq!(vertices.len(), (24 * 3 + 1) * 7);
        assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
        assert!(vertices.iter().all(|v| v.position.iter().all(|c| c.is_finite())));
        // Wound outward, as its normals face: seen from outside, not
        // through.
        for t in indices.chunks_exact(3) {
            let at = |i: u32| Vec3::from_array(vertices[i as usize].position);
            let wound = (at(t[1]) - at(t[0])).cross(at(t[2]) - at(t[0]));
            let normal = Vec3::from_array(vertices[t[0] as usize].normal);
            assert!(wound.dot(normal) > 0.0, "a face wound inward");
        }
    }

    #[test]
    fn the_same_wind_swings_it_the_same_way() {
        let run = || {
            let mut s = RopeState::new(Rope::default());
            settle(&mut s, Mat4::IDENTITY, Wind { direction: Vec3::Z, strength: 2.0 }, &[]);
            s.points().to_vec()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn a_rope_reads_as_a_line_writes_it() {
        let rope: Rope = ron::from_str("(to: (6.0, 0.0, 0.0), kind: Chain, ends: Start, stiffness: 0.5)").unwrap();
        assert_eq!(rope.stiffness, Some(0.5));
        assert_eq!(rope.kind, RopeKind::Chain);
        assert_eq!(rope.ends, Ends::Start);
        assert_eq!(rope.segments, Rope::default().segments);
        let text = ron::to_string(&Rope::default()).unwrap();
        assert!(!text.contains("end:"), "no end written when tied to nothing: {text}");
    }
}
