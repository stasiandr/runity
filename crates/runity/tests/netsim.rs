//! Simulations over the network (docs/netsim.md), on the facade's bench:
//! several peers in one process over a bad link, each running the whole
//! fixed step.

#![cfg(all(feature = "net", feature = "physics", feature = "soft"))]

use runity::glam::Vec3;
use runity::net::wire::Conditions;
use runity::netsim::bench::{Session, HZ};
use runity::scene::{Body, BodyProps, Collider, EntityDesc, Scene, Transform};
use runity::soft::{Rope, RopeKind, RopeState};
use runity::{EntityId, EntityRef};

const POST: u64 = 10;
const LOAD: u64 = 11;
/// The chain's length, metres.
const LENGTH: f32 = 1.5;

/// A chain from a post with a 10 kg box on its end, let go level with
/// the post: a pendulum.
fn pendulum(net: runity::netsim::NetMode) -> Scene {
    let mut scene = Scene::default();
    scene.entities.push(
        EntityDesc {
            id: EntityId::from_raw(1),
            name: "floor".into(),
            transform: Transform { position: Vec3::new(0.0, -0.5, 0.0), ..Default::default() },
            ..Default::default()
        }
        .with(Body::Static)
        .with(Collider::Box { half: Vec3::new(20.0, 0.5, 20.0), center: Vec3::ZERO }),
    );
    scene.entities.push(
        EntityDesc {
            id: EntityId::from_raw(POST),
            name: "post".into(),
            transform: Transform { position: Vec3::new(0.0, 3.0, 0.0), ..Default::default() },
            ..Default::default()
        }
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
        .with(BodyProps { density: 10.0 / (size.x * size.y * size.z), ..Default::default() }),
    );
    scene
}

/// What one peer shows: the chain's points and the load.
#[derive(Clone, Default)]
struct Seen {
    points: Vec<Vec3>,
    load: Vec3,
}

fn seen(session: &Session) -> Vec<Seen> {
    session
        .peers
        .iter()
        .map(|p| {
            let rope = p.entity(POST).unwrap();
            let load = p.entity(LOAD).unwrap();
            Seen {
                points: p.world.get::<&RopeState>(rope).map(|s| s.points().to_vec()).unwrap_or_default(),
                load: p.world.get::<&Transform>(load).map(|t| t.position).unwrap_or_default(),
            }
        })
        .collect()
}

fn length(points: &[Vec3]) -> f32 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// The biggest change from one tick to the next of the chain's middle,
/// less what the swing itself moves it: a jump.
fn jumps(track: &[Vec3]) -> f32 {
    track.windows(3).map(|w| (w[2] - 2.0 * w[1] + w[0]).length()).fold(0.0, f32::max)
}

#[test]
fn a_session_of_one_swings_as_the_game_alone() {
    // Alone is a session with one participant (DNA, postulate 4): the
    // chain and its load go exactly as they do with no network at all.
    let scene = pendulum(runity::netsim::NetMode::Full);
    let mut alone = Session::new(&scene, 1, Conditions::GOOD, 1);
    let mut world = hecs::World::new();
    runity::spawn_scene(&scene, &mut world, |_| Some(runity::render::MeshHandle::TEST));
    runity::world::apply_hierarchy(&mut world);
    let mut physics = runity::physics::PhysicsWorld::new(1.0 / HZ);
    physics.sync_from_world(&mut world);
    let load = runity::net::addressable(&world)[&EntityId::from_raw(LOAD)];
    for _ in 0..60 {
        alone.step();
        physics.run(&mut world);
        runity::world::apply_hierarchy(&mut world);
        runity::netsim::anchor_ropes(&mut world, &mut physics);
        runity::soft::step(&mut world, 1.0 / HZ);
        runity::netsim::pull_bodies(&mut world, &mut physics);
    }
    let there = world.get::<&Transform>(load).unwrap().position;
    let here = seen(&alone)[0].load;
    assert!(there.distance(here) < 1e-4, "{here} vs {there}");
    // And it swung, held on the chain.
    assert!(here.y < 2.0, "swung down: {here}");
    assert!(here.distance(Vec3::new(0.0, 3.0, 0.0)) < LENGTH * 1.06);
}

#[test]
fn a_chain_with_a_load_handed_over_three_times_mid_swing_never_jumps_or_drops_it() {
    let scene = pendulum(runity::netsim::NetMode::Full);
    // The solo run: how much the chain's middle changes speed in a tick
    // when nothing is wrong.
    let mut solo = Session::new(&scene, 1, Conditions::GOOD, 1);
    let mut solo_middle = Vec::new();
    for _ in 0..150 {
        solo.step();
        let s = &seen(&solo)[0];
        solo_middle.push(s.points[s.points.len() / 2]);
    }
    let solo_jump = jumps(&solo_middle);

    let mut session = Session::new(&scene, 2, Conditions::POOR, 7);
    assert!(session.join(), "everyone in: {:?}", session.problems());
    let mut tracks: Vec<Vec<Vec3>> = vec![Vec::new(); 2];
    let mut loads: Vec<Vec<Vec3>> = vec![Vec::new(); 2];
    let mut worst_length = 0.0f32;
    let mut worst_attach = 0.0f32;
    for tick in 0..150 {
        // Guest, host, guest: each takes the chain in mid-swing.
        match tick {
            20 | 80 => session.claim(1, POST),
            50 => session.claim(0, POST),
            _ => {}
        }
        session.step();
        for (peer, s) in seen(&session).iter().enumerate() {
            if s.points.is_empty() {
                continue;
            }
            tracks[peer].push(s.points[s.points.len() / 2]);
            loads[peer].push(s.load);
            if tick > 10 {
                worst_length = worst_length.max((length(&s.points) - LENGTH).abs() / LENGTH);
                worst_attach = worst_attach.max(s.load.distance(*s.points.last().unwrap()));
            }
        }
    }
    // The load went with the chain every time.
    let chain = session.owners(POST);
    assert_eq!(chain, session.owners(LOAD), "the load is driven with the chain");
    assert_eq!(chain, vec![false, true], "the guest has it last");
    assert!(worst_length < 0.05, "the chain keeps its length on every peer: {worst_length}");
    assert!(worst_attach < 0.06, "the load stays on its end on every peer: {worst_attach}");
    // A handover shows the taker the present rather than the moment the
    // picture was behind: the load, a body, jumps by its speed times the
    // delay, as every body handed over does. The chain may do as much,
    // and no more.
    for (peer, track) in tracks.iter().enumerate() {
        let (jump, load) = (jumps(track), jumps(&loads[peer]));
        assert!(jump < solo_jump.max(load) + 0.02, "peer {peer}: the chain jumped {jump} m; the load {load}, alone {solo_jump}");
    }
    assert!(session.problems().is_empty(), "{:?}", session.problems());
}

