//! Crumbling: a mud-brick wall, a pillar, a heap of crates coming down —
//! `crumble: (at: 3.0, pieces: (6, 4, 2))` on a box-shaped entity breaks it
//! at that second of its life into so many blocks, knocked outward from
//! where it was struck in a cloud of dust. The blocks fall and tumble as
//! bodies, and after a while each crumbles in turn into a little heap of
//! sand where it lay.
//!
//! The wall is a box (a scaled cube) of its material; the blocks are its
//! cells, each a little smaller and shifted, so the break is ragged.
//!
//! When it comes down is by its clock — the same moment on every machine.
//! Where each block flies is each machine's own physics (DNA, postulate 4:
//! a game must not stand on a block; they are gone to sand in seconds, and
//! what matters to play — the wall is no longer there — is the same
//! everywhere).

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::scene::{Body, Collider, Transform};
use crate::world::{LiveMesh, Model, Physics, Shape, Surface, WorldTransform};

/// A thing that comes down, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Crumble {
    /// Seconds after it is placed that it comes down; below 0, never by
    /// itself (a game brings it down with [`CrumbleState::strike`]).
    pub at: f32,
    /// Blocks along its x, y and z.
    pub pieces: [u32; 3],
    /// How hard the blocks are knocked, metres a second.
    pub burst: f32,
    /// Where the blow comes from, in its own space: `(0, 0, 1)` from in
    /// front. Zero bursts it outward from its middle.
    pub from: [f32; 3],
    /// Seconds a block lies before it crumbles into sand.
    pub lasts: f32,
}

impl Default for Crumble {
    fn default() -> Self {
        Self {
            at: 2.0,
            pieces: [5, 4, 2],
            burst: 3.0,
            from: [0.0, 0.0, 1.0],
            lasts: 5.0,
        }
    }
}

/// Seconds a block takes to crumble away once it starts.
const CRUMBLING: f32 = 1.4;
/// Seconds a puff of dust lasts.
const DUST_LIFE: f32 = 3.0;

#[derive(Debug, Clone)]
struct Block {
    entity: hecs::Entity,
    age: f32,
    /// The knock it has yet to be given: its body is made on the next
    /// physics step.
    knock: Option<Vec3>,
    size: Vec3,
    crumbling: bool,
}

#[derive(Debug, Clone, Copy)]
struct Dust {
    at: Vec3,
    drift: Vec3,
    age: f32,
    size: f32,
}

/// A thing that comes down, as it goes: the component [`run_crumble`]
/// steps.
#[derive(Debug, Clone)]
pub struct CrumbleState {
    pub crumble: Crumble,
    /// Seconds since it was placed.
    pub clock: f32,
    struck: bool,
    fallen: bool,
    blocks: Vec<Block>,
    dust: Vec<Dust>,
    color: [f32; 3],
    /// The heaps its blocks have gone to: where, and the entity.
    heaps: Vec<(Vec3, hecs::Entity)>,
}

impl CrumbleState {
    pub fn new(crumble: Crumble) -> Self {
        Self {
            crumble,
            clock: 0.0,
            struck: false,
            fallen: false,
            blocks: Vec::new(),
            dust: Vec::new(),
            color: [0.7, 0.55, 0.4],
            heaps: Vec::new(),
        }
    }

    /// Bring it down on the next step, whatever its clock says.
    pub fn strike(&mut self) {
        self.struck = true;
    }

    /// Whether it has come down.
    pub fn fallen(&self) -> bool {
        self.fallen
    }

    /// The dust in the air, for the frame's fog.
    pub fn puffs(&self) -> impl Iterator<Item = crate::volume::Puff> + '_ {
        let c = self.color;
        self.dust.iter().map(move |d| {
            let t = (d.age / DUST_LIFE).clamp(0.0, 1.0);
            crate::volume::Puff {
                position: d.at,
                radius: d.size * (0.6 + 1.2 * t.sqrt()),
                density: 3.0 * (1.0 - t) * (1.0 - t),
                color: c,
            }
        })
    }
}

/// A number in [0, 1) from two others.
fn hashed(a: u64, b: u64) -> f32 {
    let mut x = a.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ b.wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    x ^= x >> 31;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 29;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// Step everything that crumbles by `seconds`: bring down what is due,
/// knock its blocks once they have bodies, crumble the old ones to sand.
/// Call it once a fixed step, after the physics.
pub fn run_crumble(world: &mut hecs::World, physics: &mut crate::PhysicsWorld, seconds: f32) {
    let mut due = Vec::new();
    for (entity, state) in world.query_mut::<(hecs::Entity, &mut CrumbleState)>() {
        state.clock += seconds;
        let by_clock = state.crumble.at >= 0.0 && state.clock >= state.crumble.at;
        if !state.fallen && (state.struck || by_clock) {
            state.fallen = true;
            due.push(entity);
        }
    }
    for entity in due {
        bring_down(world, entity);
    }
    // Knocks, ageing, crumbling away.
    let states: Vec<hecs::Entity> = world
        .query::<(hecs::Entity, &CrumbleState)>()
        .iter()
        .map(|(e, _)| e)
        .collect();
    for owner in states {
        let Ok(mut state) = world.get::<&mut CrumbleState>(owner).map(|s| (*s).clone()) else {
            continue;
        };
        for d in &mut state.dust {
            d.age += seconds;
            d.at += d.drift * seconds;
            d.drift *= (1.0 - seconds * 0.8).max(0.0);
        }
        state.dust.retain(|d| d.age < DUST_LIFE);
        let mut gone = Vec::new();
        let mut heaps = Vec::new();
        for block in &mut state.blocks {
            block.age += seconds;
            if let Some(knock) = block.knock {
                if physics.velocity(world, block.entity).is_some() {
                    physics.set_velocity(world, block.entity, knock);
                    block.knock = None;
                }
            }
            let lasts = state.crumble.lasts.max(0.0);
            if block.age >= lasts && !block.crumbling {
                // Done with physics: it stays where it lay, and goes.
                block.crumbling = true;
                let _ = world.insert_one(block.entity, Physics(Body::None));
                if let Ok(t) = world.get::<&Transform>(block.entity) {
                    state.dust.push(Dust {
                        at: t.position,
                        drift: Vec3::new(0.0, 0.3, 0.0),
                        age: DUST_LIFE * 0.4,
                        size: block.size.max_element() * 0.8,
                    });
                }
            }
            if block.crumbling {
                let t = ((block.age - lasts) / CRUMBLING).clamp(0.0, 1.0);
                if let Ok(mut transform) = world.get::<&mut Transform>(block.entity) {
                    // It slumps: down more than across.
                    transform.scale = block.size * Vec3::new(1.0 - t * 0.6, 1.0 - t, 1.0 - t * 0.6);
                    if t >= 1.0 {
                        let volume = block.size.x * block.size.y * block.size.z;
                        let foot = transform.position - Vec3::Y * block.size.y * 0.5;
                        heaps.push((foot, volume * 0.35));
                    }
                }
                if t >= 1.0 {
                    gone.push(block.entity);
                }
            }
        }
        state.blocks.retain(|b| !gone.contains(&b.entity));
        for entity in gone {
            let _ = world.despawn(entity);
        }
        // Each block's sand goes to the heap it fell by, or starts one:
        // the rubble slumps into a few mounds, not a heap per block.
        let sand = state.color;
        for (foot, volume) in heaps {
            let near = state
                .heaps
                .iter()
                .find(|(at, _)| Vec3::new(at.x - foot.x, 0.0, at.z - foot.z).length() < 1.6)
                .map(|&(_, e)| e);
            if let Some(heap) = near.and_then(|e| world.get::<&mut crate::heap::HeapState>(e).ok().map(|_| e)) {
                if let Ok(mut h) = world.get::<&mut crate::heap::HeapState>(heap) {
                    h.heap.start += volume;
                    h.heap.most += volume;
                }
                continue;
            }
            let heap = crate::heap::Heap {
                rate: 0.0,
                start: volume,
                most: volume,
                angle_deg: 24.0,
            };
            let mut material = crate::Material::new(sand[0], sand[1], sand[2]);
            material.smoothness = 0.05;
            let at = Vec3::new(foot.x, foot.y.max(0.0), foot.z);
            let entity = world.spawn((
                Transform {
                    position: at,
                    ..Transform::default()
                },
                WorldTransform(Mat4::from_translation(at)),
                Surface(material),
                crate::heap::HeapState::new(heap),
                LiveMesh::new(Vec::new(), Vec::new()),
            ));
            state.heaps.push((at, entity));
        }
        if let Ok(mut live) = world.get::<&mut CrumbleState>(owner) {
            *live = state;
        }
    }
}

/// The thing breaks: it goes, and its blocks come in its place.
fn bring_down(world: &mut hecs::World, entity: hecs::Entity) {
    let Ok(placed) = world.get::<&WorldTransform>(entity).map(|w| w.0) else {
        return;
    };
    let Ok(model) = world.get::<&Model>(entity).map(|m| m.0) else {
        return;
    };
    let material = world
        .get::<&Surface>(entity)
        .map(|s| s.0)
        .unwrap_or_default();
    let Ok(settings) = world.get::<&CrumbleState>(entity).map(|s| s.crumble) else {
        return;
    };
    let _ = world.remove_one::<Model>(entity);
    let _ = world.insert_one(entity, Physics(Body::None));
    let (scale, turn, middle) = placed.to_scale_rotation_translation();
    let [nx, ny, nz] = settings.pieces.map(|n| n.clamp(1, 12));
    let from = Vec3::from_array(settings.from);
    let seed = entity.to_bits().get();
    let mut blocks = Vec::new();
    let mut dust = Vec::new();
    let mut k = 0u64;
    for j in 0..ny {
        for i in 0..nx {
            for l in 0..nz {
                k += 1;
                let r = |n: u64| hashed(seed, k * 13 + n);
                let cell = Vec3::new(1.0 / nx as f32, 1.0 / ny as f32, 1.0 / nz as f32);
                // Its place in the unit box, a little shifted: ragged.
                let at = Vec3::new(
                    (i as f32 + 0.5) * cell.x - 0.5,
                    (j as f32 + 0.5) * cell.y - 0.5,
                    (l as f32 + 0.5) * cell.z - 0.5,
                ) + (Vec3::new(r(1), r(2), r(3)) - 0.5) * cell * 0.15;
                let shrink = 0.86 + 0.12 * r(4);
                let size = cell * scale * shrink;
                let position = placed.transform_point3(at);
                let tilt = Quat::from_euler(glam::EulerRot::XYZ, (r(5) - 0.5) * 0.2, (r(6) - 0.5) * 0.2, (r(7) - 0.5) * 0.2);
                // Knocked from the blow's side, outward, and up a little —
                // the top ones hardest, the bottom ones hardly.
                let away = if from.length_squared() > 1e-6 {
                    -from.normalize() + at * 0.8
                } else {
                    at.normalize_or(Vec3::Y)
                };
                let height = j as f32 / (ny.max(2) - 1) as f32;
                let knock = turn * away.normalize_or(Vec3::Y)
                    * settings.burst.max(0.0)
                    * (0.4 + 0.6 * height)
                    * (0.6 + 0.6 * r(8))
                    + Vec3::Y * settings.burst.max(0.0) * 0.3 * r(9);
                let mut solid = crate::scene::BodyProps::default();
                solid.density = 1.8;
                solid.friction = 0.8;
                let mut placed_block = Transform {
                    position,
                    scale: size,
                    ..Transform::default()
                };
                placed_block.set_rotation(turn * tilt);
                let block = world.spawn((
                    placed_block,
                    WorldTransform(Mat4::from_scale_rotation_translation(size, turn * tilt, position)),
                    Model(model),
                    Surface(material),
                    Physics(Body::Dynamic),
                    Shape(Collider::Box {
                        half: Vec3::splat(0.5),
                        center: Vec3::ZERO,
                    }),
                    crate::world::Props(solid),
                ));
                blocks.push(Block {
                    entity: block,
                    age: 0.0,
                    knock: Some(knock),
                    size,
                    crumbling: false,
                });
                if k % 2 == 0 {
                    dust.push(Dust {
                        at: position,
                        drift: knock * 0.25 + Vec3::Y * 0.4,
                        age: 0.0,
                        size: size.max_element() * 1.2,
                    });
                }
            }
        }
    }
    let _ = middle;
    let color = material.color();
    if let Ok(mut state) = world.get::<&mut CrumbleState>(entity) {
        state.blocks = blocks;
        state.dust = dust;
        state.color = [color.x, color.y, color.z];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wall_comes_down_at_its_second_into_blocks_that_fly_fall_and_go_to_sand() {
        let mut world = hecs::World::new();
        let wall_at = Transform {
            position: Vec3::new(0.0, 1.5, 0.0),
            scale: Vec3::new(3.0, 3.0, 0.4),
            ..Transform::default()
        };
        let floor = world.spawn((
            Transform {
                position: Vec3::new(0.0, -0.5, 0.0),
                scale: Vec3::new(40.0, 1.0, 40.0),
                ..Transform::default()
            },
            WorldTransform(Mat4::from_scale_rotation_translation(
                Vec3::new(40.0, 1.0, 40.0),
                Quat::IDENTITY,
                Vec3::new(0.0, -0.5, 0.0),
            )),
            Physics(Body::Static),
            Shape(Collider::Box {
                half: Vec3::splat(0.5),
                center: Vec3::ZERO,
            }),
        ));
        let wall = world.spawn((
            wall_at,
            WorldTransform(wall_at.matrix()),
            Model(crate::MeshHandle::TEST),
            Surface(crate::Material::new(0.7, 0.5, 0.35)),
            CrumbleState::new(Crumble {
                at: 1.0,
                pieces: [4, 3, 1],
                lasts: 3.0,
                ..Crumble::default()
            }),
        ));
        let _ = floor;
        let mut physics = crate::PhysicsWorld::new(1.0 / 60.0);
        let step = |world: &mut hecs::World, physics: &mut crate::PhysicsWorld| {
            physics.run(world);
            run_crumble(world, physics, 1.0 / 60.0);
        };
        let blocks = |world: &hecs::World| {
            world
                .query::<(&Model, &Physics)>()
                .iter()
                .filter(|(_, p)| p.0 == Body::Dynamic)
                .count()
        };
        for _ in 0..50 {
            step(&mut world, &mut physics);
        }
        assert_eq!(blocks(&world), 0, "standing until its second");
        assert!(world.get::<&Model>(wall).is_ok());
        for _ in 0..30 {
            step(&mut world, &mut physics);
        }
        assert!(world.get::<&Model>(wall).is_err(), "the wall is gone");
        assert_eq!(blocks(&world), 12, "in its blocks");
        let dusty = world.get::<&CrumbleState>(wall).unwrap().puffs().count();
        assert!(dusty > 0, "in a cloud of dust");
        // A second on: knocked away from the blow, and fallen.
        for _ in 0..90 {
            step(&mut world, &mut physics);
        }
        let flown: Vec<Vec3> = world
            .query::<(&Transform, &Physics, &Model)>()
            .iter()
            .filter(|(_, p, _)| p.0 == Body::Dynamic)
            .map(|(t, _, _)| t.position)
            .collect();
        let mean = flown.iter().copied().sum::<Vec3>() / flown.len() as f32;
        assert!(mean.z < -0.3, "knocked from in front, away: {mean}");
        assert!(mean.y < 1.4, "fallen: {mean}");
        // Then each crumbles to sand, and heaps are left.
        for _ in 0..(60 * 5) {
            step(&mut world, &mut physics);
        }
        assert_eq!(blocks(&world), 0, "all gone to sand");
        let heaps: Vec<f32> = world
            .query::<&crate::heap::HeapState>()
            .iter()
            .map(|h| h.heap.start)
            .collect();
        assert!(!heaps.is_empty() && heaps.len() < 12, "a few mounds, not a heap a block: {}", heaps.len());
        let sand: f32 = heaps.iter().sum();
        let wall = 3.0 * 3.0 * 0.4;
        assert!(sand > wall * 0.15 && sand < wall * 0.4, "its sand: {sand}");
    }
}
