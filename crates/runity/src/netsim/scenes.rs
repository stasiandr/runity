//! The bench's scenes (docs/netsim.md): what the tests play and the net
//! reel films, built in code — each one what a stage of the plan is
//! checked by.

use glam::Vec3;

use crate::scene::{Body, BodyProps, Collider, EntityDesc, Scene, Transform};
use crate::soft::{Rope, RopeKind};
use crate::{EntityId, EntityRef};

/// `x.pipe(f)` is `f(x)`: for building a line in one expression.
trait Pipe: Sized {
    fn pipe(self, f: impl FnOnce(Self) -> Self) -> Self {
        f(self)
    }
}

impl Pipe for EntityDesc {}

/// A line in a colour (sRGB), to be told apart on film. What it draws —
/// its model, or the rope or cloth or ragdoll on it — is in it.
fn paint(desc: EntityDesc, srgb: [u8; 3]) -> EntityDesc {
    desc.with(crate::look::MaterialRef::Inline(crate::material::Material::from_srgb(srgb[0], srgb[1], srgb[2])))
}

/// The floor: a slab `2·hx` by `2·hz` metres, its top at 0, drawn as a
/// plane.
fn floor(hx: f32, hz: f32) -> EntityDesc {
    let mut desc = EntityDesc {
        id: EntityId::from_raw(1),
        name: "floor".into(),
        transform: Transform { position: Vec3::new(0.0, -0.5, 0.0), ..Default::default() },
        ..Default::default()
    }
    .with(Body::Static)
    .with(Collider::Box { half: Vec3::new(hx, 0.5, hz), center: Vec3::ZERO });
    desc.children.push(
        EntityDesc {
            id: EntityId::from_raw(1001),
            name: "floor look".into(),
            transform: Transform { position: Vec3::new(0.0, 0.5, 0.0), scale: Vec3::new(hx * 2.0, 1.0, hz * 2.0), ..Default::default() },
            ..Default::default()
        }
        .with(crate::scene::ModelRef("builtin:plane".into()))
        // The builtin chequer: what moves over it is seen to.
        .with(crate::look::MaterialRef::Named("grid".into())),
    );
    desc
}

/// A body given something to see: a cube `size` across in `srgb`, a child
/// that follows it and meets nothing (the tests need no models; film
/// does).
fn shown(mut desc: EntityDesc, size: Vec3, srgb: [u8; 3]) -> EntityDesc {
    let id = EntityId::from_raw(desc.id.raw() + 1000);
    desc.children.push(paint(
        EntityDesc { id, name: format!("{} look", desc.name), transform: Transform { scale: size, ..Default::default() }, ..Default::default() }
            .with(crate::scene::ModelRef("builtin:cube".into())),
        srgb,
    ));
    desc
}

pub const POST: u64 = 10;
pub const LOAD: u64 = 11;
/// The chain's length, metres.
pub const LENGTH: f32 = 1.5;
/// A chain from a post with a 10 kg box on its end, let go level with
/// the post: a pendulum.
pub fn pendulum(net: crate::netsim::NetMode) -> Scene {
    let mut scene = Scene::default();
    scene.entities.push(
        floor(20.0, 20.0),
    );
    scene.entities.push(
        EntityDesc {
            id: EntityId::from_raw(POST),
            name: "post".into(),
            transform: Transform { position: Vec3::new(0.0, 3.0, 0.0), ..Default::default() },
            ..Default::default()
        }
        .with(crate::look::MaterialRef::Inline(crate::material::Material { metallic: 1.0, ..crate::material::Material::from_srgb(95, 98, 105) }))
        .with(Rope {
            to: Vec3::ZERO,
            end: EntityRef::to(EntityId::from_raw(LOAD)),
            kind: RopeKind::Chain,
            slack: 0.0,
            segments: 16,
            thickness: 0.03,
            net,
            ..Default::default()
        }),
    );
    let size = Vec3::splat(0.3);
    scene.entities.push(
        EntityDesc {
            id: EntityId::from_raw(LOAD),
            name: "load".into(),
            transform: Transform { position: Vec3::new(LENGTH, 3.0, 0.0), ..Default::default() },
            ..Default::default()
        }
        .with(Body::Dynamic)
        .with(Collider::Box { half: size * 0.5, center: Vec3::ZERO })
        .with(BodyProps { density: 10.0 / (size.x * size.y * size.z), ..Default::default() })
        .pipe(|d| shown(d, size, [190, 70, 55])),
    );
    scene
}
pub const PAWN_A: u64 = 20;
pub const PAWN_B: u64 = 21;
/// How far apart the two start, metres; the rope a little longer.
pub const APART: f32 = 2.4;
/// Two players' bodies on the ground with a rope between them, held in
/// both hands: A's body carries the rope, tied at its other end to B's.
pub fn tug(net: crate::netsim::NetMode) -> Scene {
    let mut scene = Scene::default();
    scene.entities.push(
        floor(40.0, 40.0),
    );
    let size = Vec3::splat(0.6);
    let density = 40.0 / (size.x * size.y * size.z);
    for (id, name, x) in [(PAWN_A, "a", -APART * 0.5), (PAWN_B, "b", APART * 0.5)] {
        let mut desc = EntityDesc {
            id: EntityId::from_raw(id),
            name: name.into(),
            transform: Transform { position: Vec3::new(x, 0.3, 0.0), ..Default::default() },
            ..Default::default()
        }
        .with(Body::Dynamic)
        .with(Collider::Box { half: size * 0.5, center: Vec3::ZERO })
        .with(BodyProps { density, ..Default::default() });
        desc = shown(desc, size, if id == PAWN_A { [60, 110, 200] } else { [225, 140, 40] });
        if id == PAWN_A {
            // The rope is drawn in its line's colour.
            desc = paint(desc, [200, 170, 110]);
            desc = desc.with(Rope {
                to: Vec3::ZERO,
                end: EntityRef::to(EntityId::from_raw(PAWN_B)),
                kind: RopeKind::Rope,
                slack: 0.03,
                segments: 24,
                thickness: 0.03,
                net,
                ..Default::default()
            });
        }
        scene.entities.push(desc);
    }
    scene
}
pub const RUNNER: u64 = 30;
pub const CAPE: u64 = 31;
pub const TAIL: u64 = 32;
/// A player's body running round a circle, a cape on its back and a tail
/// behind: the Rough and Local things everyone simulates for themselves.
pub fn runner(cape: crate::netsim::NetMode) -> Scene {
    use crate::soft::{Cloth, Ends, Pinned};
    let mut scene = Scene::default();
    scene.entities.push(
        floor(40.0, 40.0),
    );
    let size = Vec3::new(0.5, 1.6, 0.4);
    let body = EntityDesc {
        id: EntityId::from_raw(RUNNER),
        name: "runner".into(),
        transform: Transform { position: Vec3::new(3.0, 0.8, 0.0), ..Default::default() },
        ..Default::default()
    }
    .with(Body::Kinematic)
    .with(Collider::Box { half: size * 0.5, center: Vec3::ZERO });
    let mut body = shown(body, size, [120, 90, 170]);
    body.children.push(
        EntityDesc {
            id: EntityId::from_raw(CAPE),
            name: "cape".into(),
            transform: Transform { position: Vec3::new(0.0, 0.7, -0.45), ..Default::default() },
            ..Default::default()
        }
        .with(Cloth { size: [0.7, 1.0], cells: [7, 10], pinned: Pinned::Top, net: cape, ..Default::default() })
        .pipe(|d| paint(d, [200, 40, 45])),
    );
    body.children.push(
        EntityDesc {
            id: EntityId::from_raw(TAIL),
            name: "tail".into(),
            transform: Transform { position: Vec3::new(0.0, -0.3, -0.45), ..Default::default() },
            ..Default::default()
        }
        .with(Rope { to: Vec3::new(0.0, 0.0, -0.8), ends: Ends::Start, segments: 12, thickness: 0.03, ..Default::default() })
        .pipe(|d| paint(d, [120, 80, 50])),
    );
    scene.entities.push(body);
    scene
}
pub const WALL: u64 = 40;
/// A wall that breaks by itself a second in: `Event`, so its owner (the
/// host) breaks it and tells everyone how.
#[cfg(feature = "destruction")]
pub fn wall() -> Scene {
    use crate::destruction::Fracture;
    let mut scene = Scene::default();
    scene.entities.push(
        floor(40.0, 40.0),
    );
    scene.entities.push(
        EntityDesc {
            id: EntityId::from_raw(WALL),
            name: "wall".into(),
            transform: Transform { position: Vec3::new(0.0, 1.0, 0.0), scale: Vec3::new(0.3, 2.0, 2.0), ..Default::default() },
            ..Default::default()
        }
        .with(crate::scene::ModelRef("builtin:cube".into()))
        .with(Body::Static)
        .with(Collider::Box { half: Vec3::splat(0.5), center: Vec3::ZERO })
        .with(Fracture { pieces: 12, at: Some(1.0), knock: 1.5, seed: 4, net: crate::netsim::NetMode::Event, ..Default::default() })
        .pipe(|d| paint(d, [170, 80, 60])),
    );
    scene
}
pub const PERSON: u64 = 50;
#[cfg(feature = "character")]
pub fn person() -> Scene {
    use crate::character::{Ragdoll, Mode, Drive};
    let mut scene = Scene::default();
    scene.entities.push(
        floor(40.0, 40.0),
    );
    scene.entities.push(
        EntityDesc { id: EntityId::from_raw(PERSON), name: "person".into(), ..Default::default() }
            .with(Ragdoll { mode: Mode::Active, drive: Drive::Stand, recover: 4.0, net: crate::netsim::NetMode::Full, ..Default::default() })
            .pipe(|d| paint(d, [215, 170, 140])),
    );
    scene
}
pub const CRATE_A: u64 = 60;
pub const CRATE_B: u64 = 61;
pub fn crates() -> Scene {
    let mut scene = Scene::default();
    scene.entities.push(
        floor(40.0, 40.0),
    );
    let size = Vec3::splat(0.6);
    for (id, x) in [(CRATE_A, -2.5), (CRATE_B, 2.5)] {
        scene.entities.push(
            EntityDesc {
                id: EntityId::from_raw(id),
                name: format!("crate {id}"),
                transform: Transform { position: Vec3::new(x, 0.3, 0.0), ..Default::default() },
                ..Default::default()
            }
            .with(Body::Dynamic)
            .with(Collider::Box { half: size * 0.5, center: Vec3::ZERO })
            .with(BodyProps { density: 10.0 / (size.x * size.y * size.z), ..Default::default() })
            .pipe(|d| shown(d, size, if id == CRATE_A { [150, 105, 60] } else { [80, 150, 90] })),
        );
    }
    scene
}
