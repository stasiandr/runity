//! The physics bench: a stretch of physics played alone, then by several
//! players passing its bodies around over a link that gets worse rung by
//! rung, and the difference measured — the dacha simulator's
//! `Assets/Tests/PlayMode/PhysicsBench`, its thirteen scenarios and its
//! numbers, on scrap's own physics and networking (docs/NEXT_STEPS.md,
//! «Физика выдерживает «Дачу»»).
//!
//! **Three kinds of number, kept apart because each has a different fix.**
//! The *truth* — the pose on the machine simulating a body — against the
//! solo run: where it leaves the solo path, the networking changed the
//! physics itself. What *the others saw* against the truth's recent path:
//! being shown the right path late is interpolation working and costs
//! nothing; being shown a path the truth never took is what costs. And
//! the *launch detectors*, which compare nothing: the fastest any machine
//! solved a body against the fastest the solo run did, how far a copy
//! jumped in a tick, the jump in the truth at a handover, anything that
//! left the bounds.
//!
//! **Chaotic is a property of the scenario.** A cannon shot is a parabola
//! and is compared tick for tick; a stack knocked over is not — a tenth of
//! a millimetre in the first contact is a different pile — so a chaotic
//! scenario is judged on its invariants and how far its outcome drifted.
//!
//! **Real time.** The links below delay by the clock, as a real one does,
//! so a run with peers plays at the speed of its ticks; a solo run has no
//! link and goes as fast as it can.

#[allow(unused_imports)]
use crate::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use web_time::{Duration, Instant};

use glam::{Quat, Vec3};

use crate::components::Components;
use crate::id::EntityId;
use crate::net::wire::{Conditions, Laggy, Loopback, Transport};
use crate::net::{Owned, OwnershipPending, PeerId, Replica};
use crate::party::{Event, Party};
use crate::physics::PhysicsWorld;
use crate::scene::{Body as BodyKind, BodyProps, Collider, EntityDesc, Joint, Scene, Transform};

/// Ticks a second, as the game's fixed step.
pub const HZ: f32 = 30.0;

/// One loose body: a box resting where it is placed.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    pub id: u64,
    pub name: &'static str,
    pub position: Vec3,
    pub rotation: Quat,
    pub size: Vec3,
    pub mass: f32,
}

impl Body {
    fn crate_at(id: u64, name: &'static str, position: Vec3) -> Self {
        Self {
            id,
            name,
            position,
            rotation: Quat::IDENTITY,
            size: Vec3::splat(0.6),
            mass: 10.0,
        }
    }
}

/// Something done to a body at a tick, by whichever machine simulates it
/// then — only the owner may push a body, as in the game.
#[derive(Debug, Clone, PartialEq)]
pub struct Shove {
    pub tick: usize,
    /// Pushed again every tick up to here: held at a speed, carried.
    pub until: usize,
    /// Index into the scenario's bodies.
    pub body: usize,
    /// Put here first: a cannon reloading.
    pub reset: Option<(Vec3, Quat)>,
    /// The velocity it leaves with, assigned: a launch, not a nudge.
    pub velocity: Option<Vec3>,
    pub spin: Option<Vec3>,
}

impl Shove {
    fn at(tick: usize, body: usize) -> Self {
        Self {
            tick,
            until: tick,
            body,
            reset: None,
            velocity: None,
            spin: None,
        }
    }
}

/// What holds a body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JointKind {
    /// A door or a lever: an axis through the anchor.
    Hinge { anchor: Vec3, axis: Vec3 },
    /// A rope with give.
    Spring { stiffness: f32, damping: f32 },
    /// Welded.
    Fixed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenchJoint {
    pub body: usize,
    /// The other end's index, or `None` for the world.
    pub to: Option<usize>,
    pub kind: JointKind,
    /// Newtons; `None` never breaks.
    pub breaks: Option<f32>,
}

/// A piece of the level: static and the same everywhere.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub position: Vec3,
    pub size: Vec3,
    pub rotation: Quat,
}

/// How the players pass bodies around while a scenario plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    /// Only the host simulates anything: the control.
    None,
    /// Every body to the next peer in turn every `period`.
    Rotor,
    /// Claims at random, about `rate` a body a second, one in ten raced.
    Random,
    /// One body each, claimed once at the start.
    Split,
    /// Two players grab the same body every `period`, in the same tick.
    Contested,
}

/// A stretch of physics to play alone and then over a link.
#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    pub name: &'static str,
    pub chaotic: bool,
    /// How long it plays, after the warm-up.
    pub seconds: f32,
    /// No claims after this: the end is quiet.
    pub ownership_until: f32,
    /// How far the truth may leave the solo run, metres; 0 reports only.
    pub truth_budget: f32,
    pub ownership: Ownership,
    pub period: f32,
    pub rate: f32,
    /// Half the box nothing may leave.
    pub bounds: Vec3,
    pub bodies: Vec<Body>,
    pub shoves: Vec<Shove>,
    pub blocks: Vec<Block>,
    pub joints: Vec<BenchJoint>,
}

fn tick_at(seconds: f32) -> usize {
    (seconds * HZ).round() as usize
}

impl Scenario {
    fn new(
        name: &'static str,
        chaotic: bool,
        seconds: f32,
        until: f32,
        ownership: Ownership,
    ) -> Self {
        Self {
            name,
            chaotic,
            seconds,
            ownership_until: until,
            truth_budget: 0.0,
            ownership,
            period: 1.0,
            rate: 1.0,
            bounds: Vec3::new(60.0, 40.0, 60.0),
            bodies: Vec::new(),
            shoves: Vec::new(),
            blocks: Vec::new(),
            joints: Vec::new(),
        }
    }

    pub fn ticks(&self) -> usize {
        tick_at(self.seconds)
    }

    /// The thirteen, in the dacha simulator's order.
    pub fn all() -> Vec<Scenario> {
        vec![
            Self::cannon(),
            Self::slide(),
            Self::stacks(),
            Self::drop(),
            Self::ramp(),
            Self::collide(),
            Self::tug(),
            Self::carry(),
            Self::spin(),
            Self::steal(),
            Self::hinge(),
            Self::spring(),
            Self::linked(),
        ]
    }

    /// A crate fired every three seconds at 15 m/s, 35° up, spinning, and
    /// handed on every second — most shots change hands in the air.
    pub fn cannon() -> Self {
        let mut s = Self::new("cannon", false, 12.0, 8.0, Ownership::Rotor);
        s.truth_budget = 3.0;
        s.bodies
            .push(Body::crate_at(1, "crate", Vec3::new(0.0, 0.3, 0.0)));
        let angle = 35f32.to_radians();
        for shot in 0..3 {
            s.shoves.push(Shove {
                reset: Some((Vec3::new(0.0, 1.5, 0.0), Quat::IDENTITY)),
                velocity: Some(Vec3::new(angle.cos(), angle.sin(), 0.0) * 15.0),
                spin: Some(Vec3::new(0.0, 2.0, 5.0)),
                ..Shove::at(tick_at(0.5 + shot as f32 * 3.0), 0)
            });
        }
        s
    }

    /// Kicked along the floor and handed on every 0.3 s while it slides.
    pub fn slide() -> Self {
        let mut s = Self::new("slide", false, 7.0, 4.0, Ownership::Rotor);
        s.truth_budget = 1.5;
        s.period = 0.3;
        s.bodies
            .push(Body::crate_at(1, "crate", Vec3::new(0.0, 0.3, 0.0)));
        s.shoves.push(Shove {
            velocity: Some(Vec3::new(9.0, 0.0, 1.0)),
            ..Shove::at(tick_at(0.5), 0)
        });
        s.shoves.push(Shove {
            velocity: Some(Vec3::new(-8.0, 2.0, -1.0)),
            spin: Some(Vec3::new(0.0, 6.0, 0.0)),
            ..Shove::at(tick_at(2.5), 0)
        });
        s
    }

    /// Two towers of three rammed by a seventh, claims landing on every
    /// crate at random: a stack of mixed ownership.
    pub fn stacks() -> Self {
        let mut s = Self::new("stacks", true, 10.0, 6.0, Ownership::Random);
        let size = 0.5;
        let names = [
            "tower0-0", "tower0-1", "tower0-2", "tower1-0", "tower1-1", "tower1-2",
        ];
        for tower in 0..2 {
            for level in 0..3 {
                let id = (tower * 3 + level + 1) as u64;
                s.bodies.push(Body {
                    size: Vec3::splat(size),
                    mass: 8.0,
                    ..Body::crate_at(
                        id,
                        names[id as usize - 1],
                        Vec3::new(
                            3.0,
                            size * 0.5 + level as f32 * size,
                            tower as f32 * 0.55 - 0.275,
                        ),
                    )
                });
            }
        }
        s.bodies.push(Body {
            size: Vec3::splat(0.7),
            mass: 25.0,
            ..Body::crate_at(7, "ram", Vec3::new(-2.0, 0.35, 0.0))
        });
        s.shoves.push(Shove {
            velocity: Some(Vec3::new(10.0, 1.5, 0.0)),
            ..Shove::at(tick_at(1.0), 6)
        });
        s
    }

    /// Dropped from six metres and handed on twice on the way down.
    pub fn drop() -> Self {
        let mut s = Self::new("drop", false, 9.0, 6.0, Ownership::Rotor);
        s.truth_budget = 1.0;
        s.period = 0.4;
        s.bodies
            .push(Body::crate_at(1, "crate", Vec3::new(0.0, 0.3, 0.0)));
        for drop in 0..3 {
            s.shoves.push(Shove {
                reset: Some((Vec3::new(drop as f32 * 1.5, 6.0, 0.0), Quat::IDENTITY)),
                velocity: Some(Vec3::new(0.5, 0.0, 0.0)),
                spin: Some(Vec3::new(0.5, 0.0, 1.5)),
                ..Shove::at(tick_at(0.5 + drop as f32 * 2.0), 0)
            });
        }
        s
    }

    /// Let go at the top of a 35° slab and handed on every half second
    /// all the way down: a sliding contact that never lets up.
    pub fn ramp() -> Self {
        let mut s = Self::new("ramp", false, 15.0, 8.0, Ownership::Rotor);
        s.truth_budget = 2.0;
        s.period = 0.5;
        let tilt = Quat::from_rotation_z(-35f32.to_radians());
        let slab = Vec3::new(16.0, 0.5, 6.0);
        s.blocks.push(Block {
            position: Vec3::new(0.0, 4.0, 0.0),
            size: slab,
            rotation: tilt,
        });
        let along = tilt * Vec3::X;
        let up = tilt * Vec3::Y;
        let at = Vec3::new(0.0, 4.0, 0.0) - along * 6.0 + up * (slab.y * 0.5 + 0.32);
        s.bodies.push(Body {
            rotation: tilt,
            ..Body::crate_at(1, "crate", at)
        });
        // Nudged off, clear of the first claim.
        s.shoves.push(Shove {
            velocity: Some(along * 3.0),
            ..Shove::at(tick_at(0.3), 0)
        });
        s
    }

    /// Two crates, one each, shoved head-on into each other: a contact
    /// across the ownership boundary.
    pub fn collide() -> Self {
        let mut s = Self::new("collide", true, 8.0, 1.0, Ownership::Split);
        s.bodies
            .push(Body::crate_at(1, "mine", Vec3::new(-2.5, 0.3, 0.0)));
        s.bodies
            .push(Body::crate_at(2, "theirs", Vec3::new(2.5, 0.3, 0.0)));
        s.shoves.push(Shove {
            velocity: Some(Vec3::new(9.0, 0.0, 0.0)),
            ..Shove::at(tick_at(1.5), 0)
        });
        s.shoves.push(Shove {
            velocity: Some(Vec3::new(-9.0, 0.0, 0.2)),
            ..Shove::at(tick_at(1.5), 1)
        });
        s
    }

    /// Two players grabbing the same crate twice a second, in one tick.
    pub fn tug() -> Self {
        let mut s = Self::new("tug", false, 9.0, 6.0, Ownership::Contested);
        s.truth_budget = 1.5;
        s.period = 0.5;
        s.bodies
            .push(Body::crate_at(1, "crate", Vec3::new(0.0, 0.3, 0.0)));
        s.shoves.push(Shove {
            velocity: Some(Vec3::new(4.0, 0.5, 0.5)),
            ..Shove::at(tick_at(1.0), 0)
        });
        s.shoves.push(Shove {
            velocity: Some(Vec3::new(-3.0, 1.0, -0.5)),
            ..Shove::at(tick_at(4.0), 0)
        });
        s
    }

    /// Carried at 4 m/s for four seconds, changing hands three times.
    pub fn carry() -> Self {
        let mut s = Self::new("carry", false, 9.0, 5.0, Ownership::Rotor);
        s.truth_budget = 2.0;
        s.period = 1.2;
        s.bodies
            .push(Body::crate_at(1, "crate", Vec3::new(-5.0, 0.3, 0.0)));
        s.shoves.push(Shove {
            until: tick_at(5.0),
            velocity: Some(Vec3::new(4.0, 0.0, 0.0)),
            ..Shove::at(tick_at(1.0), 0)
        });
        s
    }

    /// A plank thrown spinning hard and handed on while it tumbles.
    pub fn spin() -> Self {
        let mut s = Self::new("spin", false, 9.0, 6.0, Ownership::Rotor);
        s.truth_budget = 1.5;
        s.period = 0.6;
        s.bodies.push(Body {
            size: Vec3::new(1.6, 0.2, 0.4),
            mass: 6.0,
            ..Body::crate_at(1, "plank", Vec3::new(0.0, 0.3, 0.0))
        });
        for throw in 0..2 {
            s.shoves.push(Shove {
                reset: Some((
                    Vec3::new(throw as f32 * 2.0 - 4.0, 3.0, 0.0),
                    Quat::IDENTITY,
                )),
                velocity: Some(Vec3::new(5.0, 3.0, 0.0)),
                spin: Some(Vec3::new(0.0, 0.0, 14.0)),
                ..Shove::at(tick_at(0.8 + throw as f32 * 3.5), 0)
            });
        }
        s
    }

    /// Kicked on the very tick another player takes it, four times.
    pub fn steal() -> Self {
        let mut s = Self::new("steal", false, 9.0, 6.0, Ownership::Rotor);
        s.truth_budget = 1.5;
        s.bodies
            .push(Body::crate_at(1, "crate", Vec3::new(0.0, 0.3, 0.0)));
        for kick in 1..=4 {
            let x = if kick % 2 == 0 { -5.0 } else { 5.0 };
            s.shoves.push(Shove {
                velocity: Some(Vec3::new(x, 0.5, 0.0)),
                ..Shove::at(tick_at(kick as f32), 0)
            });
        }
        s
    }

    /// A lever on a hinge, kicked twice and handed on while it swings.
    pub fn hinge() -> Self {
        let mut s = Self::new("hinge", false, 10.0, 7.0, Ownership::Rotor);
        s.truth_budget = 1.0;
        s.period = 0.7;
        s.bodies.push(Body {
            size: Vec3::new(1.5, 0.2, 0.3),
            mass: 8.0,
            ..Body::crate_at(1, "lever", Vec3::new(0.75, 2.0, 0.0))
        });
        s.joints.push(BenchJoint {
            body: 0,
            to: None,
            kind: JointKind::Hinge {
                anchor: Vec3::new(-0.75, 0.0, 0.0),
                axis: Vec3::Z,
            },
            breaks: None,
        });
        s.shoves.push(Shove {
            spin: Some(Vec3::new(0.0, 0.0, 6.0)),
            ..Shove::at(tick_at(0.7), 0)
        });
        s.shoves.push(Shove {
            spin: Some(Vec3::new(0.0, 0.0, -7.0)),
            ..Shove::at(tick_at(4.3), 0)
        });
        s
    }

    /// A crate on a spring, pulled hard enough to tear it off.
    pub fn spring() -> Self {
        let mut s = Self::new("spring", false, 10.0, 7.0, Ownership::Rotor);
        s.truth_budget = 1.5;
        s.period = 0.5;
        s.bodies
            .push(Body::crate_at(1, "bob", Vec3::new(0.0, 2.0, 0.0)));
        s.joints.push(BenchJoint {
            body: 0,
            to: None,
            kind: JointKind::Spring {
                stiffness: 200.0,
                damping: 2.0,
            },
            breaks: Some(600.0),
        });
        s.shoves.push(Shove {
            velocity: Some(Vec3::new(3.0, 0.0, 0.0)),
            ..Shove::at(tick_at(1.0), 0)
        });
        s.shoves.push(Shove {
            velocity: Some(Vec3::new(11.0, 2.0, 0.0)),
            ..Shove::at(tick_at(5.0), 0)
        });
        s
    }

    /// Two crates welded together, one owned by each player: a joint
    /// straight across the ownership boundary.
    pub fn linked() -> Self {
        let mut s = Self::new("linked", false, 9.0, 1.0, Ownership::Split);
        s.truth_budget = 2.0;
        s.bodies
            .push(Body::crate_at(1, "left", Vec3::new(-0.4, 0.3, 0.0)));
        s.bodies
            .push(Body::crate_at(2, "right", Vec3::new(0.4, 0.3, 0.0)));
        s.joints.push(BenchJoint {
            body: 0,
            to: Some(1),
            kind: JointKind::Fixed,
            breaks: None,
        });
        s.shoves.push(Shove {
            until: tick_at(4.0),
            velocity: Some(Vec3::new(3.0, 0.0, 0.0)),
            ..Shove::at(tick_at(2.0), 0)
        });
        s.shoves.push(Shove {
            until: tick_at(7.0),
            velocity: Some(Vec3::new(-3.0, 0.0, 0.0)),
            ..Shove::at(tick_at(5.0), 1)
        });
        s
    }

    /// The scene every peer loads: the floor, the blocks, the bodies and
    /// their joints.
    pub fn scene(&self) -> Scene {
        let mut entities = vec![EntityDesc {
            id: EntityId::from_raw(1000),
            name: "floor".into(),
            transform: Transform {
                position: Vec3::new(0.0, -0.5, 0.0),
                ..Default::default()
            },
            ..Default::default()
        }
        .with(BodyKind::Static)
        .with(Collider::Box {
            half: Vec3::new(80.0, 0.5, 80.0),
            center: Vec3::ZERO,
        })];
        for (i, block) in self.blocks.iter().enumerate() {
            let mut transform = Transform {
                position: block.position,
                ..Default::default()
            };
            transform.set_rotation(block.rotation);
            entities.push(
                EntityDesc {
                    id: EntityId::from_raw(1001 + i as u64),
                    name: format!("block{i}"),
                    transform,
                    ..Default::default()
                }
                .with(BodyKind::Static)
                .with(Collider::Box {
                    half: block.size * 0.5,
                    center: Vec3::ZERO,
                }),
            );
        }
        for (i, body) in self.bodies.iter().enumerate() {
            let mut transform = Transform {
                position: body.position,
                ..Default::default()
            };
            transform.set_rotation(body.rotation);
            let volume = body.size.x * body.size.y * body.size.z;
            let mut desc = EntityDesc {
                id: EntityId::from_raw(body.id),
                name: body.name.into(),
                transform,
                ..Default::default()
            }
            .with(BodyKind::Dynamic)
            .with(Collider::Box {
                half: body.size * 0.5,
                center: Vec3::ZERO,
            })
            .with(BodyProps {
                density: body.mass / volume.max(1e-6),
                ..Default::default()
            });
            for joint in self.joints.iter().filter(|j| j.body == i) {
                let to = joint
                    .to
                    .map(|t| EntityId::from_raw(self.bodies[t].id))
                    .unwrap_or_default();
                let made = match joint.kind {
                    JointKind::Fixed => Joint::Fixed { to },
                    JointKind::Hinge { anchor, axis } => Joint::Hinge {
                        to,
                        anchor,
                        axis,
                        limits_deg: None,
                        motor: None,
                    },
                    JointKind::Spring { stiffness, damping } => Joint::Spring {
                        to,
                        anchor: Vec3::ZERO,
                        stiffness,
                        damping,
                    },
                };
                desc.set_part(&made);
                desc.set_joint_break(joint.breaks);
            }
            entities.push(desc);
        }
        Scene {
            entities,
            ..Default::default()
        }
    }
}

/// A rung of the ladder: a link, named.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rung {
    pub name: &'static str,
    pub link: Conditions,
}

/// Perfect to awful, as the dacha simulator's ladder.
pub fn ladder() -> Vec<Rung> {
    let ms = Duration::from_millis;
    vec![
        Rung {
            name: "perfect",
            link: Conditions::GOOD,
        },
        Rung {
            name: "lan",
            link: Conditions {
                latency: ms(2),
                jitter: ms(1),
                loss: 0.0,
                duplicate: 0.0,
            },
        },
        Rung {
            name: "broadband",
            link: Conditions {
                latency: ms(20),
                jitter: ms(5),
                loss: 0.005,
                duplicate: 0.0,
            },
        },
        Rung {
            name: "poor",
            link: Conditions::POOR,
        },
        Rung {
            name: "awful",
            link: Conditions::AWFUL,
        },
    ]
}

/// One run's choices.
#[derive(Debug, Clone)]
pub struct Settings {
    pub peers: usize,
    pub rung: Rung,
    pub seed: u64,
    /// Overrides the scenario's; a solo reference passes `None`.
    pub ownership: Option<Ownership>,
    pub warmup: f32,
}

impl Settings {
    pub fn solo() -> Self {
        Self {
            peers: 1,
            rung: ladder()[0],
            seed: 1,
            ownership: Some(Ownership::None),
            warmup: 2.0,
        }
    }

    pub fn on(rung: Rung, seed: u64) -> Self {
        Self {
            peers: 3,
            rung,
            seed,
            ownership: None,
            warmup: 2.0,
        }
    }
}

/// One body on one peer at one tick.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Sample {
    pub valid: bool,
    pub position: Vec3,
    /// The solver's; zero on a copy.
    pub velocity: Vec3,
    /// A copy moved by someone else's poses.
    pub kinematic: bool,
    pub owned: bool,
    pub pending: bool,
}

/// Everything every peer showed and solved, tick by tick.
#[derive(Debug, Clone)]
pub struct Recording {
    pub scenario: Scenario,
    pub link: &'static str,
    pub seed: u64,
    pub peers: usize,
    pub ticks: usize,
    /// `[tick][peer][body]`.
    pub samples: Vec<Vec<Vec<Sample>>>,
    pub joined: bool,
    pub claims_planned: usize,
    pub pushed: usize,
    pub problems: Vec<String>,
    /// What the guests sent, and what the server sent them, bytes.
    pub sent: u64,
    pub served: u64,
}

impl Recording {
    fn at(&self, tick: usize, peer: usize, body: usize) -> Sample {
        self.samples[tick][peer][body]
    }
}

/// Counts what goes through a transport.
struct Metered<T> {
    inner: T,
    sent: Arc<AtomicU64>,
}

impl<T: Transport> Transport for Metered<T> {
    fn send(&mut self, to: PeerId, bytes: Vec<u8>) {
        self.sent.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        self.inner.send(to, bytes);
    }

    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        self.inner.receive()
    }
}

struct Peer {
    party: Party,
    world: hecs::World,
    physics: PhysicsWorld,
    bodies: Vec<hecs::Entity>,
}

impl Peer {
    fn new(party: Party, scenario: &Scenario) -> Self {
        let mut world = hecs::World::new();
        crate::world::spawn_scene_dressed(
            &scenario.scene(),
            &mut world,
            &mut [Box::new(scrap_physics::PhysicsDress)],
        );
        crate::world::apply_hierarchy(&mut world);
        let mut physics = PhysicsWorld::new(1.0 / HZ);
        physics.sync_from_world(&mut world);
        let addressed = crate::net::addressable(&world);
        let bodies = scenario
            .bodies
            .iter()
            .map(|b| addressed[&EntityId::from_raw(b.id)])
            .collect();
        Self {
            party,
            world,
            physics,
            bodies,
        }
    }

    /// One tick: what the network says, the pushes this peer may give
    /// (it owns the body), then the physics. How many it gave.
    fn tick(
        &mut self,
        components: &Components,
        now: usize,
        shoves: &[&Shove],
        problems: &mut Vec<String>,
    ) -> usize {
        for event in self
            .party
            .update(&mut self.world, components, 1.0 / HZ, |_, _, _| None)
        {
            if let Event::Problem(problem) = event {
                problems.push(problem);
            }
        }
        crate::world::apply_hierarchy(&mut self.world);
        // Bodies built by the first run, so pushes land on them.
        self.physics.sync_from_world(&mut self.world);
        let mut pushed = 0;
        for shove in shoves {
            let entity = self.bodies[shove.body];
            if self.world.get::<&Owned>(entity).is_err() {
                continue;
            }
            // A held push is the same push again, without its teleport.
            if let Some((at, turn)) = shove.reset.filter(|_| shove.tick == now) {
                self.physics.teleport(&mut self.world, entity, at, turn);
                // Settled as a move of the scene's before the push, or
                // the next sync would take it for one and stop it.
                crate::world::apply_hierarchy(&mut self.world);
                self.physics.sync_from_world(&mut self.world);
            }
            if let Some(v) = shove.velocity {
                self.physics.set_velocity(&self.world, entity, v);
            }
            if let Some(w) = shove.spin {
                self.physics.set_spin(&self.world, entity, w);
            }
            pushed += 1;
        }
        self.physics.run(&mut self.world);
        pushed
    }

    fn sample(&self, body: usize) -> Sample {
        let entity = self.bodies[body];
        let Ok(transform) = self.world.get::<&Transform>(entity) else {
            return Sample::default();
        };
        let kinematic = self.world.get::<&Replica>(entity).is_ok();
        Sample {
            valid: true,
            position: transform.position,
            velocity: if kinematic {
                Vec3::ZERO
            } else {
                self.physics
                    .velocity(&self.world, entity)
                    .unwrap_or(Vec3::ZERO)
            },
            kinematic,
            owned: self.world.get::<&Owned>(entity).is_ok(),
            pending: self.world.get::<&OwnershipPending>(entity).is_ok(),
        }
    }
}

/// Who claims what, when: `(tick, peer, body)`, in tick order.
fn plan(scenario: &Scenario, settings: &Settings) -> Vec<(usize, usize, usize)> {
    let peers = settings.peers;
    let mode = settings.ownership.unwrap_or(scenario.ownership);
    let until = scenario.ticks().min(tick_at(scenario.ownership_until));
    let bodies = scenario.bodies.len();
    let mut out = Vec::new();
    if peers < 2 || mode == Ownership::None {
        return out;
    }
    let period = tick_at(scenario.period).max(1);
    match mode {
        Ownership::None => {}
        Ownership::Split => {
            for b in 0..bodies {
                out.push((1, b % peers, b));
            }
        }
        Ownership::Rotor | Ownership::Contested => {
            let mut turn = 1;
            while turn * period < until {
                for b in 0..bodies {
                    out.push((turn * period, turn % peers, b));
                    if mode == Ownership::Contested {
                        out.push((turn * period, (turn + 1) % peers, b));
                    }
                }
                turn += 1;
            }
        }
        Ownership::Random => {
            let mut seed = settings.seed.wrapping_mul(7919).wrapping_add(17) | 1;
            let mut random = move || {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                (seed >> 11) as f64 / (1u64 << 53) as f64
            };
            let chance = scenario.rate as f64 / HZ as f64;
            for tick in 0..until {
                for b in 0..bodies {
                    if random() >= chance {
                        continue;
                    }
                    let first = (random() * peers as f64) as usize % peers;
                    out.push((tick, first, b));
                    if random() < 0.1 {
                        let other = (first + 1 + (random() * (peers - 1) as f64) as usize) % peers;
                        out.push((tick, other, b));
                    }
                }
            }
        }
    }
    out.sort_by_key(|c| c.0);
    out
}

/// Play one scenario on `settings.peers` peers — the host and guests over
/// the rung's link — and record everything.
pub fn play(scenario: &Scenario, settings: &Settings) -> Recording {
    let components = Components::new();
    let sent = Arc::new(AtomicU64::new(0));
    let served = Arc::new(AtomicU64::new(0));
    let mut ends = Loopback::network(settings.peers as u32).into_iter();
    let listener = ends.next().expect("the host's end");
    let mut peers = vec![Peer::new(
        Party::host(
            "bench",
            "host",
            &components,
            vec![Box::new(Metered {
                inner: listener,
                sent: served.clone(),
            })],
            false,
        ),
        scenario,
    )];
    for (i, end) in ends.enumerate() {
        let lagged: Box<dyn Transport + Send> = if settings.rung.link == Conditions::GOOD {
            Box::new(Metered {
                inner: end,
                sent: sent.clone(),
            })
        } else {
            Box::new(Metered {
                inner: Laggy::new(end, settings.rung.link, settings.seed * 101 + i as u64),
                sent: sent.clone(),
            })
        };
        peers.push(Peer::new(
            Party::join(lagged, "bench", &format!("guest{}", i + 1), &components),
            scenario,
        ));
    }
    let networked = settings.peers > 1;
    let tick_length = Duration::from_secs_f32(1.0 / HZ);
    let mut problems = Vec::new();
    let mut clock = Instant::now();
    let pace = |clock: &mut Instant| {
        if networked {
            *clock += tick_length;
            let now = Instant::now();
            if *clock > now {
                std::thread::sleep(*clock - now);
            }
        }
    };
    // Everyone in, then the warm-up.
    let mut joined = false;
    for _ in 0..tick_at(8.0) {
        for peer in &mut peers {
            peer.tick(&components, usize::MAX, &[], &mut problems);
        }
        pace(&mut clock);
        if peers.iter().all(|p| p.party.welcomed()) {
            joined = true;
            break;
        }
    }
    for _ in 0..tick_at(settings.warmup) {
        for peer in &mut peers {
            peer.tick(&components, usize::MAX, &[], &mut problems);
        }
        pace(&mut clock);
    }
    let ticks = scenario.ticks();
    let claims = plan(scenario, settings);
    let mut next = 0;
    let mut pushed = 0;
    let mut samples = Vec::with_capacity(ticks);
    for tick in 0..ticks {
        while next < claims.len() && claims[next].0 == tick {
            let (_, p, b) = claims[next];
            let peer = &mut peers[p];
            let entity = peer.bodies[b];
            peer.party.claim(&mut peer.world, entity);
            next += 1;
        }
        let shoves: Vec<&Shove> = scenario
            .shoves
            .iter()
            .filter(|s| tick >= s.tick && tick <= s.until.max(s.tick))
            .collect();
        for peer in &mut peers {
            pushed += peer.tick(&components, tick, &shoves, &mut problems);
        }
        samples.push(
            peers
                .iter()
                .map(|p| (0..scenario.bodies.len()).map(|b| p.sample(b)).collect())
                .collect(),
        );
        pace(&mut clock);
    }
    Recording {
        scenario: scenario.clone(),
        link: settings.rung.name,
        seed: settings.seed,
        peers: settings.peers,
        ticks,
        samples,
        joined,
        claims_planned: claims.len(),
        pushed,
        problems,
        sent: sent.load(Ordering::Relaxed),
        served: served.load(Ordering::Relaxed),
    }
}

/// How far back a viewer's pose is matched against the truth's path.
const VIEW_WINDOW: f32 = 1.0;
/// A viewer's pose moving further than this in a tick is a teleport.
const TELEPORT_METRES: f32 = 1.5;
/// Faster than the solo run went, by this factor plus the slack, is a
/// launch.
pub const LAUNCH_FACTOR: f32 = 1.25;
pub const LAUNCH_SLACK: f32 = 2.0;
/// At rest, every screen agrees to within this.
pub const CONVERGENCE_METRES: f32 = 0.05;
/// What the others see stays this close to the truth's path, 95% of
/// the time.
pub const VIEW_P95_METRES: f32 = 0.5;
/// Below this movement a tick, the truth has settled.
pub const SETTLED_METRES_PER_TICK: f32 = 0.01;

/// What one run measured.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Result {
    pub scenario: String,
    pub link: String,
    pub seed: u64,
    pub peers: usize,
    pub joined: bool,
    pub problems: usize,
    pub first_problem: Option<String>,
    pub claims_planned: usize,
    pub pushed: usize,
    /// What a guest put on the wire a second, and what the server sent
    /// out a second.
    pub bytes_per_second: f32,
    pub served_per_second: f32,
    pub handovers: usize,
    pub dual_simulation_ticks: usize,
    pub truth_error_mean: f32,
    pub truth_error_max: f32,
    pub outcome_error: f32,
    pub view_error_mean: f32,
    pub view_error_p95: f32,
    pub view_error_max: f32,
    pub view_lag_ms: f32,
    pub view_teleports: usize,
    pub reference_max_speed: f32,
    pub max_dynamic_speed: f32,
    pub copy_jump_max: f32,
    pub handover_jump_mean: f32,
    pub handover_jump_max: f32,
    pub out_of_bounds: usize,
    pub convergence: f32,
    pub rest_jitter: f32,
    pub rest_motion: f32,
    pub truth_budget: f32,
    pub hard_pass: bool,
    pub pass: bool,
    pub verdict: String,
}

/// The body's pose on whichever machine simulated it, tick by tick, and
/// who that was: the settled owner, else an optimistic one, else whoever
/// it was last tick.
pub fn truth(run: &Recording, body: usize) -> (Vec<Vec3>, Vec<usize>) {
    let mut current = 0;
    let mut path = Vec::with_capacity(run.ticks);
    let mut owners = Vec::with_capacity(run.ticks);
    for t in 0..run.ticks {
        let owning = (0..run.peers).filter(|p| {
            let s = run.at(t, *p, body);
            s.valid && s.owned
        });
        let settled = owning.clone().find(|p| !run.at(t, *p, body).pending);
        if let Some(p) = settled.or_else(|| owning.clone().next()) {
            current = p;
        }
        owners.push(current);
        path.push(run.at(t, current, body).position);
    }
    (path, owners)
}

fn max_speed(run: &Recording) -> f32 {
    let mut max = 0.0f32;
    for tick in &run.samples {
        for peer in tick {
            for s in peer {
                if s.valid && !s.kinematic {
                    max = max.max(s.velocity.length());
                }
            }
        }
    }
    max
}

/// A run's numbers, against the solo run of the same scenario.
pub fn evaluate(run: &Recording, reference: Option<&Recording>) -> Result {
    let scenario = &run.scenario;
    let seconds = run.ticks as f32 / HZ;
    let window = (VIEW_WINDOW * HZ).round() as usize;
    let guests = run.peers.saturating_sub(1).max(1) as f32;
    let mut r = Result {
        scenario: scenario.name.into(),
        link: run.link.into(),
        seed: run.seed,
        peers: run.peers,
        joined: run.joined,
        problems: run.problems.len(),
        first_problem: run.problems.first().cloned(),
        claims_planned: run.claims_planned,
        pushed: run.pushed,
        bytes_per_second: if seconds > 0.0 {
            run.sent as f32 / seconds / guests
        } else {
            0.0
        },
        served_per_second: if seconds > 0.0 {
            run.served as f32 / seconds
        } else {
            0.0
        },
        ..Default::default()
    };
    let resets: Vec<usize> = scenario
        .shoves
        .iter()
        .filter(|s| s.reset.is_some())
        .map(|s| s.tick)
        .collect();
    let recent_reset = |t: usize| resets.iter().any(|r| t >= *r && t - r <= window);
    let mut views: Vec<f32> = Vec::new();
    let (mut lags, mut lag_count) = (0usize, 0usize);
    let (mut truth_sum, mut truth_count) = (0f64, 0usize);
    let mut jump_sum = 0f64;
    for b in 0..scenario.bodies.len() {
        let (path, owners) = truth(run, b);
        for t in 0..run.ticks {
            if t > 0 && owners[t] != owners[t - 1] {
                r.handovers += 1;
                if !resets.contains(&t) {
                    let expected = run.at(t - 1, owners[t - 1], b).velocity / HZ;
                    let jump = (path[t] - path[t - 1] - expected).length();
                    jump_sum += jump as f64;
                    r.handover_jump_max = r.handover_jump_max.max(jump);
                }
            }
            let mut simulating = 0;
            for p in 0..run.peers {
                let s = run.at(t, p, b);
                if !s.valid {
                    continue;
                }
                if !s.kinematic {
                    simulating += 1;
                    r.max_dynamic_speed = r.max_dynamic_speed.max(s.velocity.length());
                } else if t > 0 && run.at(t - 1, p, b).kinematic && !recent_reset(t) {
                    let jumped = (s.position - run.at(t - 1, p, b).position).length();
                    r.copy_jump_max = r.copy_jump_max.max(jumped);
                }
                if s.position.abs().cmpgt(scenario.bounds).any() {
                    r.out_of_bounds += 1;
                }
                if p == owners[t] {
                    continue;
                }
                let (mut best, mut best_lag) = (f32::MAX, 0);
                for k in 0..=window.min(t) {
                    let d = s.position.distance(path[t - k]);
                    if d < best {
                        best = d;
                        best_lag = k;
                    }
                }
                views.push(best);
                lags += best_lag;
                lag_count += 1;
                if t > 0
                    && !recent_reset(t)
                    && s.position.distance(run.at(t - 1, p, b).position) > TELEPORT_METRES
                {
                    r.view_teleports += 1;
                }
            }
            if simulating > 1 {
                r.dual_simulation_ticks += 1;
            }
            if let Some(reference) = reference {
                let e = path[t].distance(reference.at(t, 0, b).position);
                truth_sum += e as f64;
                truth_count += 1;
                r.truth_error_max = r.truth_error_max.max(e);
            }
        }
        let last = run.ticks - 1;
        if let Some(reference) = reference {
            r.outcome_error += path[last].distance(reference.at(last, 0, b).position);
        }
        // Still twitching over the quiet last second?
        let quiet = run.ticks.saturating_sub(HZ as usize).max(1);
        for t in quiet..run.ticks {
            for p in 0..run.peers {
                let (now, before) = (run.at(t, p, b), run.at(t - 1, p, b));
                if now.valid && before.valid {
                    r.rest_jitter = r.rest_jitter.max(now.position.distance(before.position));
                }
            }
            r.rest_motion = r.rest_motion.max(path[t].distance(path[t - 1]));
        }
        for p in 0..run.peers {
            let s = run.at(last, p, b);
            if s.valid {
                r.convergence = r.convergence.max(s.position.distance(path[last]));
            }
        }
    }
    if let Some(reference) = reference {
        r.reference_max_speed = max_speed(reference);
        r.truth_error_mean = if truth_count > 0 {
            (truth_sum / truth_count as f64) as f32
        } else {
            0.0
        };
        r.outcome_error /= scenario.bodies.len().max(1) as f32;
    }
    r.handover_jump_mean = if r.handovers > 0 {
        (jump_sum / r.handovers as f64) as f32
    } else {
        0.0
    };
    r.truth_budget = if scenario.chaotic {
        0.0
    } else {
        scenario.truth_budget
    };
    if !views.is_empty() {
        views.sort_by(f32::total_cmp);
        r.view_error_mean = views.iter().sum::<f32>() / views.len() as f32;
        r.view_error_p95 = views[((views.len() as f32 * 0.95) as usize).min(views.len() - 1)];
        r.view_error_max = views[views.len() - 1];
        r.view_lag_ms = lags as f32 / lag_count.max(1) as f32 / HZ * 1000.0;
    }
    judge(&mut r);
    r
}

fn judge(r: &mut Result) {
    let mut reasons = Vec::new();
    if !r.joined {
        reasons.push("a peer never joined".to_string());
    }
    if r.problems > 0 {
        reasons.push(format!("{} problem(s) reported", r.problems));
    }
    if r.out_of_bounds > 0 {
        reasons.push("left the bounds".into());
    }
    let launch = r.reference_max_speed * LAUNCH_FACTOR + LAUNCH_SLACK;
    if r.reference_max_speed > 0.0 && r.max_dynamic_speed > launch {
        reasons.push(format!(
            "launched: {:.1} m/s solved, solo peaked at {:.1}",
            r.max_dynamic_speed, r.reference_max_speed
        ));
    }
    // Only where the truth came to rest: a scenario still playing out has
    // screens legitimately a couple of frames apart.
    if r.rest_motion <= SETTLED_METRES_PER_TICK && r.convergence > CONVERGENCE_METRES {
        reasons.push(format!(
            "screens disagree by {:.2} m at rest",
            r.convergence
        ));
    }
    r.hard_pass = reasons.is_empty();
    if r.truth_budget > 0.0 && r.truth_error_max > r.truth_budget {
        reasons.push(format!(
            "the physics itself moved {:.2} m from the solo run",
            r.truth_error_max
        ));
    }
    if r.view_error_p95 > VIEW_P95_METRES {
        reasons.push(format!("view p95 {:.2} m", r.view_error_p95));
    }
    r.pass = reasons.is_empty();
    r.verdict = if r.pass {
        "ok".into()
    } else {
        reasons.join("; ")
    };
}

/// The results as a table, one run a line.
pub fn table(results: &[Result]) -> String {
    let mut out = String::from(
        "scenario  link       seed truthMax truthMean viewP95 lagMs jumpMax  B/s  served  verdict\n",
    );
    for r in results {
        out.push_str(&format!(
            "{:<9} {:<10} {:>4} {:>8.2} {:>9.2} {:>7.2} {:>5.0} {:>7.2} {:>5.0} {:>7.0}  {}\n",
            r.scenario,
            r.link,
            r.seed,
            r.truth_error_max,
            r.truth_error_mean,
            r.view_error_p95,
            r.view_lag_ms,
            r.handover_jump_max,
            r.bytes_per_second,
            r.served_per_second,
            r.verdict
        ));
    }
    out
}

/// How far down the ladder a scenario holds: the last rung it passed on
/// every seed with every rung above passed too.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Frontier {
    pub scenario: String,
    /// Two solo runs apart: what "the same" can mean at all.
    pub noise_floor: f32,
    pub passes: String,
    pub hard_passes: String,
}

/// A ladder's measurement.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Report {
    pub seeds: u64,
    pub frontier: Vec<Frontier>,
    pub results: Vec<Result>,
}

impl Report {
    /// The frontier, then every run.
    pub fn text(&self) -> String {
        let mut out = String::from("scenario  noise    passes     hard passes\n");
        for f in &self.frontier {
            out.push_str(&format!(
                "{:<9} {:>6.4}   {:<10} {}\n",
                f.scenario, f.noise_floor, f.passes, f.hard_passes
            ));
        }
        out.push('\n');
        out.push_str(&table(&self.results));
        out
    }
}

/// Every scenario `wanted` says yes to, on every rung it says yes to,
/// `seeds` times each, walking down the ladder. `progress` hears a line
/// per run, `i/N` first.
pub fn measure(
    seeds: u64,
    scenario_wanted: impl Fn(&str) -> bool,
    rung_wanted: impl Fn(&str) -> bool,
    mut progress: impl FnMut(&str),
) -> Report {
    let scenarios: Vec<Scenario> = Scenario::all()
        .into_iter()
        .filter(|s| scenario_wanted(s.name))
        .collect();
    let rungs: Vec<Rung> = ladder()
        .into_iter()
        .filter(|r| rung_wanted(r.name))
        .collect();
    let total = scenarios.len() * rungs.len() * seeds as usize;
    let mut done = 0;
    let mut report = Report {
        seeds,
        ..Default::default()
    };
    for scenario in &scenarios {
        let reference = play(scenario, &Settings::solo());
        let repeat = play(scenario, &Settings::solo());
        let mut frontier = Frontier {
            scenario: scenario.name.into(),
            noise_floor: evaluate(&repeat, Some(&reference)).truth_error_max,
            passes: "none".into(),
            hard_passes: "none".into(),
        };
        let (mut passing, mut hard_passing) = (true, true);
        for rung in &rungs {
            let (mut all, mut all_hard) = (true, true);
            for seed in 1..=seeds {
                let run = play(scenario, &Settings::on(*rung, seed));
                let result = evaluate(&run, Some(&reference));
                done += 1;
                progress(&format!(
                    "{done}/{total} {} {} seed {seed}: {}",
                    scenario.name, rung.name, result.verdict
                ));
                all &= result.pass;
                all_hard &= result.hard_pass;
                report.results.push(result);
            }
            passing &= all;
            hard_passing &= all_hard;
            if passing {
                frontier.passes = rung.name.into();
            }
            if hard_passing {
                frontier.hard_passes = rung.name.into();
            }
        }
        report.frontier.push(frontier);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scenario_builds_a_scene_whose_bodies_stand_still_alone() {
        for scenario in Scenario::all() {
            let run = play(&scenario, &Settings::solo());
            assert!(run.joined, "{}", scenario.name);
            assert!(run.pushed > 0, "{}: nothing was pushed", scenario.name);
            // Every push moves what it pushes.
            let moved =
                (0..run.ticks).any(|t| {
                    scenario.bodies.iter().enumerate().any(|(b, body)| {
                        run.samples[t][0][b].position.distance(body.position) > 0.5
                    })
                });
            assert!(moved, "{}: nothing moved", scenario.name);
            let result = evaluate(&run, Some(&run));
            assert_eq!(result.out_of_bounds, 0, "{}", scenario.name);
            assert_eq!(result.handovers, 0, "{}", scenario.name);
        }
    }

    #[test]
    fn a_solo_run_repeats_itself() {
        let scenario = Scenario::cannon();
        let first = play(&scenario, &Settings::solo());
        let second = play(&scenario, &Settings::solo());
        assert_eq!(first.pushed, 3, "all three shots fired");
        let result = evaluate(&second, Some(&first));
        assert!(
            result.truth_error_max < 1e-3,
            "two solo runs are the noise floor: {}",
            result.truth_error_max
        );
    }

    #[test]
    fn the_rotor_hands_the_crate_around_and_everyone_sees_it() {
        let scenario = Scenario::cannon();
        let reference = play(&scenario, &Settings::solo());
        let run = play(&scenario, &Settings::on(ladder()[0], 1));
        let result = evaluate(&run, Some(&reference));
        assert!(result.joined, "{result:#?}");
        assert!(
            result.handovers >= 6,
            "a turn a second for eight seconds round three: {result:#?}"
        );
        assert!(result.pushed >= 3, "whoever had it fired it: {result:#?}");
        assert!(result.hard_pass, "{}", table(&[result]));
    }
}
