//! Things that break: a line's `fracture`. `fracture: (pieces: 16)` on a
//! box-shaped thing — a wall, a crate, a pillar — breaks it into so many
//! Voronoi pieces when something hits it harder than `strength` metres a
//! second, or at its `at` second; each piece a dynamic body knocked away
//! from the blow, drawn with the thing's material, its cut faces too.
//!
//! * `pattern: Ahead` cuts it when it is spawned — the same pieces every
//!   time, ready when it breaks (Blast's and Chaos's precomputed
//!   fracture). `AtBlow` cuts it at the moment of the blow, the pieces
//!   small round where it was struck and large away from it.
//! * `levels: 2` and more: the pieces break again when hit again, each
//!   into fewer — a hierarchy of chunks, as Blast has it.
//! * Its own `particles`, if it has them, go off as it breaks: debris,
//!   and with `gpu: true`, thousands of grains of it.

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use runity_core::scene::Transform;
use runity_core::world::WorldTransform;
use runity_physics::bodies::{Physics, Shape};
use runity_physics::body::{Body, Collider};

use crate::voronoi::{cells, Dice, Solid};

/// A thing that breaks, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fracture {
    /// How many pieces.
    pub pieces: u32,
    /// Metres a second something must hit it at to break it.
    pub strength: f32,
    /// Or at this second of its life, whatever hits it.
    #[serde(skip_serializing_if = "Option::is_none", with = "runity_core::defaults::plain")]
    pub at: Option<f32>,
    /// Where the blow comes from when it breaks by the clock, in its own
    /// space; its middle is knocked out the other way.
    pub from: Vec3,
    /// How hard the pieces are knocked away, metres a second.
    pub knock: f32,
    /// Cut ahead, or at the blow.
    pub pattern: Pattern,
    /// How many times a piece may break again.
    pub levels: u32,
    /// Which pieces: another number, other pieces.
    pub seed: u32,
    /// How it goes over the network (docs/netsim.md): `Local` unless
    /// the line says.
    #[serde(default, skip_serializing_if = "runity_core::netsim::NetMode::is_local")]
    pub net: runity_core::netsim::NetMode,
}

/// When a fracture is cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Pattern {
    #[default]
    Ahead,
    AtBlow,
}

impl Default for Fracture {
    fn default() -> Self {
        Self {
            pieces: 16,
            strength: 4.0,
            at: None,
            from: Vec3::new(0.0, 0.0, 1.0),
            knock: 2.0,
            pattern: Pattern::Ahead,
            levels: 1,
            seed: 1,
            net: runity_core::netsim::NetMode::Local,
        }
    }
}

runity_core::impl_parts! {
    Fracture => "fracture";
}

/// The fracture of a line, read off it.
pub trait FractureLine {
    fn fracture(&self) -> Option<Fracture>;
}

impl FractureLine for runity_core::EntityDesc {
    fn fracture(&self) -> Option<Fracture> {
        self.part()
    }
}

impl FractureLine for runity_core::scene::Override {
    fn fracture(&self) -> Option<Fracture> {
        self.part()
    }
}

/// A thing that can break: what it is, and its pieces when cut ahead.
#[derive(Debug, Clone)]
pub struct Breakable {
    pub fracture: Fracture,
    /// Its shape, in its own space: the unit box, or a piece's.
    pub solid: Solid,
    /// The pieces, cut ahead.
    cut: Option<Vec<Solid>>,
    pub age: f32,
    /// Broken, and gone.
    pub broken: bool,
    /// The blow it broke by, where, which way and how hard (in the
    /// world): what an `Event` fracture tells everyone else.
    pub struck: Option<(Vec3, Vec3, f32)>,
    /// A blow it is to break by, told by the peer that broke it: broken so
    /// at the next step whatever this peer's own blows say.
    pub told: Option<(Vec3, Vec3, f32)>,
}

impl Breakable {
    pub fn new(fracture: Fracture, solid: Solid) -> Self {
        let cut = (fracture.pattern == Pattern::Ahead).then(|| cut(&solid, fracture, None));
        Self { fracture, solid, cut, age: 0.0, broken: false, struck: None, told: None }
    }

    /// The pieces it breaks into, struck at `blow` (in its own space).
    pub fn pieces(&self, blow: Option<Vec3>) -> Vec<Solid> {
        match &self.cut {
            Some(cut) => cut.clone(),
            None => cut(&self.solid, self.fracture, blow),
        }
    }
}

/// `solid` cut into its fracture's pieces: points spread through it, or,
/// struck, gathered round the blow.
fn cut(solid: &Solid, fracture: Fracture, blow: Option<Vec3>) -> Vec<Solid> {
    let (low, high) = solid.bounds();
    let mut dice = Dice(0x9e37_79b9_7f4a_7c15 ^ (fracture.seed as u64 + 1).wrapping_mul(0x2545_f491_4f6c_dd1d));
    let n = fracture.pieces.clamp(2, 128) as usize;
    let size = (high - low).length();
    let points: Vec<Vec3> = (0..n)
        .map(|i| match blow {
            // Most of them round the blow, nearer the fewer there are.
            Some(at) if i < n * 3 / 4 => {
                let r = dice.next().powf(1.5) * size * 0.45;
                let way = Vec3::new(dice.next() - 0.5, dice.next() - 0.5, dice.next() - 0.5).normalize_or(Vec3::X);
                (at + way * r).clamp(low, high)
            }
            _ => dice.within(low, high),
        })
        .collect();
    cells(solid, &points)
}

/// A piece of something broken: its shape in its own space (round its
/// middle), and the triangles it is drawn with, for the render.
#[derive(Debug, Clone)]
pub struct Piece {
    pub vertices: Vec<runity_geometry::mesh_asset::Vertex>,
    pub indices: Vec<u32>,
    /// What it broke off.
    pub from: hecs::Entity,
    /// Which piece of it: the same number for the same piece on every
    /// peer.
    pub index: u32,
    /// It has been given its look.
    pub dressed: bool,
}

/// Something struck: by what speed, where (in the world), which way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blow {
    pub entity: hecs::Entity,
    pub speed: f32,
    pub at: Vec3,
    pub way: Vec3,
}

/// What breaking made: each piece and how it should be moving, and what
/// broke.
#[derive(Debug, Default)]
pub struct Broken {
    pub pieces: Vec<(hecs::Entity, Vec3)>,
    pub things: Vec<hecs::Entity>,
}

/// Everything that can break on by `seconds`, broken where struck by
/// `blows` hard enough or by its clock; the pieces spawned as dynamic
/// bodies. The thing itself is put away (despawned).
pub fn run_fracture(world: &mut hecs::World, seconds: f32, blows: &[Blow]) -> Broken {
    let mut out = Broken::default();
    let mut due = Vec::new();
    for (entity, breakable, placed, replica) in world.query_mut::<(hecs::Entity, &mut Breakable, &WorldTransform, Option<&runity_core::world::Replica>)>() {
        if breakable.broken {
            continue;
        }
        // Told by the peer that broke it: broken the same way here.
        if let Some(blow) = breakable.told.take() {
            breakable.broken = true;
            breakable.struck = Some(blow);
            due.push((entity, placed.0, blow));
            continue;
        }
        // An `Event` fracture of someone else's waits to be told: its
        // owner's blows, not this peer's picture of them, break it.
        if replica.is_some() && breakable.fracture.net == runity_core::netsim::NetMode::Event {
            continue;
        }
        breakable.age += seconds;
        let struck = blows
            .iter()
            .filter(|b| b.entity == entity && b.speed >= breakable.fracture.strength)
            .max_by(|a, b| a.speed.total_cmp(&b.speed))
            .copied();
        let timed = breakable.fracture.at.is_some_and(|at| breakable.age >= at);
        if struck.is_none() && !timed {
            continue;
        }
        breakable.broken = true;
        let blow = struck.map(|b| (b.at, b.way.normalize_or(Vec3::NEG_Z), b.speed)).unwrap_or_else(|| {
            let from = placed.0.transform_point3(breakable.fracture.from);
            let middle = placed.0.transform_point3(Vec3::ZERO);
            (from, (middle - from).normalize_or(Vec3::NEG_Z), breakable.fracture.knock)
        });
        breakable.struck = Some(blow);
        due.push((entity, placed.0, blow));
    }
    for (entity, placed, (at, way, speed)) in due {
        let Some(breakable) = world.get::<&Breakable>(entity).ok().map(|b| (*b).clone()) else { continue };
        let local_blow = placed.inverse().transform_point3(at);
        let pieces = breakable.pieces(Some(local_blow));
        let (scale, turn, _) = placed.to_scale_rotation_translation();
        let knock = breakable.fracture.knock;
        for (index, solid) in pieces.into_iter().enumerate() {
            let (volume, centre) = solid.volume_and_centre();
            if volume <= 1e-6 {
                continue;
            }
            let (vertices, indices) = solid.mesh(centre, scale);
            let (low, high) = solid.bounds();
            let world_centre = placed.transform_point3(centre);
            // Knocked along the blow, and out from where it landed, the
            // nearer the harder.
            let out_from = (world_centre - at).normalize_or(way);
            let near = 1.0 / (1.0 + world_centre.distance(at));
            let velocity = way * (speed * 0.3 * near) + out_from * (knock * near) + Vec3::Y * knock * 0.3;
            let mut piece_solid = solid.clone();
            for face in &mut piece_solid.faces {
                for c in &mut face.corners {
                    *c -= centre;
                }
            }
            let shape = Shape(Collider::Box {
                half: ((high - low) * 0.5).max(Vec3::splat(0.01)),
                center: (low + high) * 0.5 - centre,
            });
            let mut transform = Transform { position: world_centre, scale, ..Transform::default() };
            transform.set_rotation(turn);
            let piece = world.spawn((
                transform,
                WorldTransform(Mat4::from_scale_rotation_translation(scale, turn, world_centre)),
                Physics(Body::Dynamic),
                shape,
                Piece { vertices, indices, from: entity, index: index as u32, dressed: false },
            ));
            if breakable.fracture.levels > 1 {
                let again = Fracture {
                    pieces: (breakable.fracture.pieces / 3).max(2),
                    levels: breakable.fracture.levels - 1,
                    at: None,
                    pattern: Pattern::AtBlow,
                    // By its number, not its entity: the same pieces of the
                    // same piece on every peer.
                    seed: breakable.fracture.seed.wrapping_add(index as u32 + 1),
                    ..breakable.fracture
                };
                let _ = world.insert_one(piece, Breakable::new(again, piece_solid));
            }
            out.pieces.push((piece, velocity));
        }
        out.things.push(entity);
    }
    out
}

/// The destruction module's dresser for fracture.
pub struct FractureDress;

impl runity_core::world::Dress for FractureDress {
    fn parts(&self) -> &[&'static str] {
        &["fracture"]
    }

    fn dress(
        &mut self,
        line: &runity_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: runity_core::world::Changed,
        _: &mut Vec<runity_core::world::Unresolved>,
    ) {
        match line.fracture() {
            Some(fracture) => {
                let solid = Solid::cuboid(Vec3::splat(-0.5), Vec3::splat(0.5));
                let _ = world.insert_one(entity, Breakable::new(fracture, solid));
            }
            None => {
                let _ = world.remove_one::<Breakable>(entity);
            }
        }
    }
}


/// A fracture's state for the network (`Components::register_state`):
/// the blow it broke by, from its owner, when it is `Event`.
pub fn gather_net(world: &hecs::World, entity: hecs::Entity) -> Option<Vec<u8>> {
    let breakable = world.get::<&Breakable>(entity).ok()?;
    if breakable.fracture.net != runity_core::netsim::NetMode::Event || world.get::<&runity_core::world::Replica>(entity).is_ok() {
        return None;
    }
    let (at, way, speed) = breakable.struck?;
    let mut out = Vec::with_capacity(28);
    for x in [at.x, at.y, at.z, way.x, way.y, way.z, speed] {
        out.extend_from_slice(&x.to_le_bytes());
    }
    Some(out)
}

/// The blow its owner broke it by: broken so here at the next step.
pub fn take_net(world: &mut hecs::World, entity: hecs::Entity, _sender: u32, _tick: u64, bytes: &[u8]) {
    if bytes.len() < 28 {
        return;
    }
    let f = |i: usize| f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap_or_default());
    let blow = (Vec3::new(f(0), f(1), f(2)), Vec3::new(f(3), f(4), f(5)), f(6));
    if let Ok(mut breakable) = world.get::<&mut Breakable>(entity) {
        if !breakable.broken && breakable.told.is_none() {
            breakable.told = Some(blow);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use glam::Quat;

    fn wall(world: &mut hecs::World, fracture: Fracture) -> hecs::Entity {
        let placed = Mat4::from_scale_rotation_translation(Vec3::new(4.0, 2.0, 0.3), Quat::IDENTITY, Vec3::new(0.0, 1.0, 0.0));
        world.spawn((
            WorldTransform(placed),
            Breakable::new(fracture, Solid::cuboid(Vec3::splat(-0.5), Vec3::splat(0.5))),
        ))
    }

    #[test]
    fn a_wall_holds_against_a_tap_and_breaks_under_a_blow_into_pieces_that_fill_it() {
        let mut world = hecs::World::new();
        let w = wall(&mut world, Fracture { pieces: 12, strength: 5.0, ..Fracture::default() });
        let tap = Blow { entity: w, speed: 2.0, at: Vec3::new(0.0, 1.0, 0.15), way: Vec3::NEG_Z };
        assert!(run_fracture(&mut world, 1.0 / 60.0, &[tap]).pieces.is_empty());
        let blow = Blow { speed: 9.0, ..tap };
        let broken = run_fracture(&mut world, 1.0 / 60.0, &[blow]);
        assert_eq!(broken.things, vec![w]);
        assert_eq!(broken.pieces.len(), 12);
        // Each piece a dynamic box body where its part of the wall was,
        // knocked back along the blow.
        let mut volume = 0.0;
        for (piece, velocity) in &broken.pieces {
            let placed = world.get::<&WorldTransform>(*piece).unwrap().0.w_axis.truncate();
            assert!(placed.x.abs() <= 2.0 && (0.0..=2.0).contains(&placed.y), "{placed}");
            assert!(velocity.z < 0.0, "knocked along the blow: {velocity}");
            let p = world.get::<&Piece>(*piece).unwrap();
            assert!(!p.indices.is_empty());
            // Its mesh, scaled out, is its share of the wall.
            let scaled: Vec<Vec3> = p.vertices.iter().map(|v| Vec3::from_array(v.position) * Vec3::new(4.0, 2.0, 0.3)).collect();
            for t in p.indices.chunks_exact(3) {
                let (a, b, c) = (scaled[t[0] as usize], scaled[t[1] as usize], scaled[t[2] as usize]);
                volume += a.dot(b.cross(c)) / 6.0;
            }
        }
        assert!((volume - 2.4).abs() < 0.01, "the pieces are the wall: {volume}");
        // Broken once, it does not break again.
        assert!(run_fracture(&mut world, 1.0 / 60.0, &[blow]).pieces.is_empty());
    }

    #[test]
    fn cut_at_the_blow_the_pieces_are_small_where_it_struck() {
        let mut world = hecs::World::new();
        let w = wall(&mut world, Fracture { pieces: 24, pattern: Pattern::AtBlow, ..Fracture::default() });
        let at = Vec3::new(-1.5, 1.5, 0.15);
        let broken = run_fracture(&mut world, 1.0 / 60.0, &[Blow { entity: w, speed: 10.0, at, way: Vec3::NEG_Z }]);
        let size = |e: hecs::Entity| {
            let p = world.get::<&Piece>(e).unwrap();
            let (lo, hi) = p.vertices.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), v| {
                (a.min(Vec3::from_array(v.position)), b.max(Vec3::from_array(v.position)))
            });
            ((hi - lo) * Vec3::new(4.0, 2.0, 0.3)).length()
        };
        let (mut near, mut far) = (Vec::new(), Vec::new());
        for (piece, _) in &broken.pieces {
            let placed = world.get::<&WorldTransform>(*piece).unwrap().0.w_axis.truncate();
            if placed.distance(at) < 1.0 { near.push(size(*piece)) } else { far.push(size(*piece)) }
        }
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
        assert!(!near.is_empty() && !far.is_empty());
        assert!(mean(&near) < mean(&far), "near {near:?} far {far:?}");
    }

    #[test]
    fn a_piece_breaks_again_into_fewer_and_the_clock_breaks_what_is_not_struck() {
        let mut world = hecs::World::new();
        let _ = wall(&mut world, Fracture { pieces: 6, levels: 2, at: Some(0.5), ..Fracture::default() });
        assert!(run_fracture(&mut world, 0.25, &[]).pieces.is_empty());
        let broken = run_fracture(&mut world, 0.3, &[]);
        assert_eq!(broken.pieces.len(), 6, "broken by its clock");
        let (piece, _) = broken.pieces[0];
        assert!(world.get::<&Breakable>(piece).is_ok(), "a piece can break again");
        let again = run_fracture(&mut world, 0.1, &[Blow { entity: piece, speed: 10.0, at: Vec3::new(0.0, 1.0, 0.0), way: Vec3::NEG_Z }]);
        assert_eq!(again.pieces.len(), 2);
        assert!(again.pieces.iter().all(|(p, _)| world.get::<&Breakable>(*p).is_err()), "the last level is the last");
    }
}
