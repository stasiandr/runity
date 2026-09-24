//! Simulations over the network (docs/netsim.md), on the facade's bench:
//! several peers in one process over a bad link, each running the whole
//! fixed step.

#![cfg(all(feature = "net", feature = "physics", feature = "soft"))]

use runity::glam::Vec3;
use runity::net::wire::Conditions;
use runity::netsim::bench::{ticks, Session, HZ};
use runity::netsim::scenes::*;
use runity::scene::{Scene, Transform};
use runity::soft::{Rope, RopeKind, RopeState};
use runity::{EntityId, EntityRef};



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
    for _ in 0..ticks(2.0) {
        alone.step();
        // The bench's step, but for the party.
        runity::world::apply_hierarchy(&mut world);
        runity::netsim::claim_approaching(&mut world, &physics);
        runity::character::step(&mut world, &mut physics, 1.0 / HZ);
        physics.run(&mut world);
        runity::world::apply_hierarchy(&mut world);
        runity::netsim::anchor_ropes(&mut world, &mut physics);
        runity::soft::step(&mut world, 1.0 / HZ);
        runity::netsim::pull_bodies(&mut world, &mut physics);
        runity::destruction::step(&mut world, &mut physics, 1.0 / HZ);
    }
    let there = world.get::<&Transform>(load).unwrap().position;
    let here = seen(&alone)[0].load;
    // The same, but for the order things are visited in (being `Owned`
    // puts an entity in another of the world's tables), which a chain
    // swinging for two seconds makes a few millimetres of.
    assert!(there.distance(here) < 0.02, "{here} vs {there}");
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
    for _ in 0..ticks(5.0) {
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
    for tick in 0..ticks(5.0) {
        // Guest, host, guest: each takes the chain in mid-swing.
        if tick == ticks(0.67) || tick == ticks(2.67) {
            session.claim(1, POST);
        }
        if tick == ticks(1.67) {
            session.claim(0, POST);
        }
        session.step();
        for (peer, s) in seen(&session).iter().enumerate() {
            if s.points.is_empty() {
                continue;
            }
            tracks[peer].push(s.points[s.points.len() / 2]);
            loads[peer].push(s.load);
            if tick > ticks(0.33) {
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




/// Each player pulls their own body away from the other: A with `a`
/// newtons, B with `b`. On the machine that simulates the body.
fn pull(peer: &mut runity::netsim::bench::Peer, a: f32, b: f32) {
    for (id, force) in [(PAWN_A, Vec3::new(-a, 0.0, 0.0)), (PAWN_B, Vec3::new(b, 0.0, 0.0))] {
        let Some(e) = peer.entity(id) else { continue };
        if peer.world.get::<&runity::net::Owned>(e).is_ok() {
            peer.physics.add_force(&peer.world, e, force);
        }
    }
}

/// What a peer shows of the tug: both bodies' x, the rope's length.
fn tug_seen(peer: &runity::netsim::bench::Peer) -> (f32, f32, f32) {
    let x = |id| peer.entity(id).and_then(|e| peer.world.get::<&Transform>(e).ok().map(|t| t.position.x)).unwrap_or(f32::NAN);
    let rope = peer.entity(PAWN_A).and_then(|e| peer.world.get::<&RopeState>(e).ok().map(|s| length(s.points()))).unwrap_or(0.0);
    (x(PAWN_A), x(PAWN_B), rope)
}

/// How far apart the rope's ends are on a peer.
fn ends_apart(peer: &runity::netsim::bench::Peer) -> f32 {
    peer.entity(PAWN_A)
        .and_then(|e| peer.world.get::<&RopeState>(e).ok().map(|s| {
            let p = s.points();
            p.first().zip(p.last()).map_or(0.0, |(a, b)| a.distance(*b))
        }))
        .unwrap_or(0.0)
}

/// What a tug of war came to.
#[derive(Debug, Clone, Copy)]
struct Tug {
    /// How far B got on the worse of the two peers, of how far alone.
    moved: f32,
    /// How much further apart the rope's ends are at its owner's than it
    /// is long, 95th percentile.
    over: f32,
    /// The longest it is drawn anywhere, of its length.
    longest: f32,
    /// Once everything stops: how far apart the two pictures are.
    disagree: f32,
    /// The fastest either body went on either peer.
    fastest: f32,
    /// Both saw A dragged along.
    dragged: bool,
}

fn tug_of_war(link: Conditions, seed: u64) -> Tug {
    const A: f32 = 200.0;
    const B: f32 = 700.0;
    let secs = 3.0;
    let scene = tug(runity::netsim::NetMode::Full);
    let rest = APART * 1.03;
    // Alone: both pulls on one machine.
    let mut solo = Session::new(&scene, 1, Conditions::GOOD, 1);
    for _ in 0..ticks(secs) {
        solo.step_with(|_, p| pull(p, A, B));
    }
    let (solo_a, solo_b, _) = tug_seen(&solo.peers[0]);
    assert!(solo_b > APART * 0.5 + 1.0 && solo_a > -APART * 0.5 + 1.0, "alone B wins and drags A: {solo_a} {solo_b}");

    let mut session = Session::new(&scene, 2, link, seed);
    for p in &mut session.peers {
        for id in [PAWN_A, PAWN_B] {
            let e = p.entity(id).unwrap();
            let _ = p.world.insert_one(e, runity::netsim::Pawn);
        }
    }
    assert!(session.join(), "everyone in: {:?}", session.problems());
    // B's body is the guest's.
    session.claim(1, PAWN_B);
    for _ in 0..ticks(0.5) {
        session.step();
    }
    assert_eq!(session.owners(PAWN_B), vec![false, true]);
    assert_eq!(session.owners(PAWN_A), vec![true, false], "the rope stays with A's body");
    let start: Vec<(f32, f32, f32)> = session.peers.iter().map(tug_seen).collect();
    let mut over = Vec::new();
    let mut longest = 0.0f32;
    let mut fastest = 0.0f32;
    let speeds = |session: &Session| {
        session
            .peers
            .iter()
            .flat_map(|p| [PAWN_A, PAWN_B].map(|id| p.physics.velocity(&p.world, p.entity(id).unwrap()).unwrap_or_default().length()))
            .fold(0.0f32, f32::max)
    };
    for _ in 0..ticks(secs) {
        session.step_with(|_, p| pull(p, A, B));
        over.push((ends_apart(&session.peers[0]) - rest) / rest);
        fastest = fastest.max(speeds(&session));
        for p in &session.peers {
            let (a, b, rope) = tug_seen(p);
            assert!(a.is_finite() && b.is_finite(), "nothing thrown: {a} {b}");
            longest = longest.max(rope / rest);
        }
    }
    let solo_moved = solo_b - APART * 0.5;
    let moved = session.peers.iter().enumerate().map(|(i, p)| tug_seen(p).1 - start[i].1).fold(f32::MAX, f32::min);
    let dragged = session.peers.iter().enumerate().all(|(i, p)| tug_seen(p).0 - start[i].0 > 0.5);
    over.sort_by(f32::total_cmp);
    let over95 = over[over.len() * 95 / 100];
    // Let go: once it has all stopped, where does each see everything.
    for _ in 0..ticks(2.0) {
        session.step();
        fastest = fastest.max(speeds(&session));
    }
    let (a0, b0, _) = tug_seen(&session.peers[0]);
    let (a1, b1, _) = tug_seen(&session.peers[1]);
    assert!(session.problems().is_empty(), "{:?}", session.problems());
    Tug {
        moved: moved / solo_moved,
        over: over95,
        longest,
        disagree: (a0 - a1).abs().max((b0 - b1).abs()),
        fastest,
        dragged,
    }
}

/// What a tug of war must come to: both see the stronger win and drag the
/// other about as far as alone, the rope holds its length where it is
/// simulated, and when it stops they agree.
fn tug_holds(tug: Tug) {
    eprintln!("{tug:?}");
    assert!(tug.dragged, "A dragged along on both");
    assert!(tug.moved > 0.6, "B got {} as far as alone", tug.moved);
    assert!(tug.over < 0.06, "the rope's ends at its owner's over its length by {}", tug.over);
    assert!(tug.longest < 1.6, "drawn at most {}× its length", tug.longest);
    assert!(tug.disagree < 0.25, "the pictures at rest {} apart", tug.disagree);
    assert!(tug.fastest < 12.0, "nothing flung: {} m/s", tug.fastest);
}

/// One set of the numbers of a rope held across machines, from
/// `RUNITY_ACROSS` (`taut,damping,keep,gate,catch,catch_damping,slack`),
/// on two links and two seeds each, printed: what `scripts` sweep.
#[test]
#[ignore]
fn sweep_one() {
    let Ok(text) = std::env::var("RUNITY_ACROSS") else { return };
    let v: Vec<f32> = text.split(',').map(|x| x.trim().parse().unwrap()).collect();
    *runity::netsim::ACROSS.write().unwrap() = runity::netsim::Across {
        taut: v[0],
        damping: v[1],
        keep: v[2],
        gate: v[3],
        catch: v[4],
        catch_damping: v[5],
        slack: v[6],
    };
    let bad = Conditions { latency: std::time::Duration::from_millis(75), jitter: std::time::Duration::from_millis(15), loss: 0.05, duplicate: 0.01 };
    for (name, link) in [("poor", Conditions::POOR), ("bad", bad)] {
        let seeds: Vec<u64> = std::env::var("RUNITY_SEEDS").map(|s| s.split(',').map(|x| x.parse().unwrap()).collect()).unwrap_or(vec![3, 11]);
        for seed in seeds {
            let t = tug_of_war(link, seed);
            println!("SWEEP {text} {name} {seed} moved {:.2} over {:.3} longest {:.2} disagree {:.2} fastest {:.1} dragged {}", t.moved, t.over, t.longest, t.disagree, t.fastest, t.dragged);
        }
    }
}

#[test]
fn two_players_pull_a_rope_apart_and_both_see_the_stronger_win_on_a_poor_link() {
    tug_holds(tug_of_war(Conditions::POOR, 3));
}

#[test]
fn two_players_pull_a_rope_apart_and_nothing_breaks_at_150_ms_and_5_percent_lost() {
    let bad = Conditions {
        latency: std::time::Duration::from_millis(75),
        jitter: std::time::Duration::from_millis(15),
        loss: 0.05,
        duplicate: 0.01,
    };
    tug_holds(tug_of_war(bad, 5));
}



/// Run round a circle of 3 m at 3 m/s, facing the way it goes: moved by
/// the body's owner.
fn run(peer: &mut runity::netsim::bench::Peer, tick: usize) {
    let Some(e) = peer.entity(RUNNER) else { return };
    if peer.world.get::<&runity::net::Owned>(e).is_err() {
        return;
    }
    let angle = tick as f32 / HZ;
    if let Ok(mut t) = peer.world.get::<&mut Transform>(e) {
        t.position = Vec3::new(angle.cos() * 3.0, 0.8, angle.sin() * 3.0);
        // Facing along the circle: its back (−z) behind it.
        t.set_rotation(runity::glam::Quat::from_rotation_y(-angle));
    }
}

/// Where the cape's bottom middle and the tail's tip are, in the runner's
/// space, on a peer.
fn hanging(peer: &runity::netsim::bench::Peer) -> (Vec3, Vec3) {
    let body = peer.entity(RUNNER).unwrap();
    let back = peer.world.get::<&runity::world::WorldTransform>(body).unwrap().0.inverse();
    let cape = peer.entity(CAPE).and_then(|e| peer.world.get::<&runity::soft::ClothState>(e).ok().map(|s| {
        let (w, h) = s.grid();
        s.points()[(h - 1) * w + w / 2]
    }));
    let tail = peer.entity(TAIL).and_then(|e| peer.world.get::<&RopeState>(e).ok().and_then(|s| s.points().last().copied()));
    (back.transform_point3(cape.unwrap_or_default()), back.transform_point3(tail.unwrap_or_default()))
}

/// The root mean square gap between two tracks, the second shifted by up
/// to eight ticks either way, at the best shift: how unlike two peers'
/// pictures of one thing are, the one's delay aside.
fn unlike(a: &[Vec3], b: &[Vec3]) -> f32 {
    unlike_within(a, b, 8)
}

/// [`unlike`], shifting up to `most` ticks either way.
fn unlike_within(a: &[Vec3], b: &[Vec3], most: isize) -> f32 {
    (0..=2 * most)
        .map(|shift| {
            let s = shift - most;
            let pairs: Vec<f32> = (0..a.len())
                .filter_map(|i| {
                    let j = i as isize + s;
                    (j >= 0 && (j as usize) < b.len()).then(|| a[i].distance_squared(b[j as usize]))
                })
                .collect();
            (pairs.iter().sum::<f32>() / pairs.len().max(1) as f32).sqrt()
        })
        .fold(f32::MAX, f32::min)
}

/// A cape and a tail on a runner owned by the guest, as the host sees
/// them over `link`: how much they shake on each peer (the biggest change
/// of speed from a tick to the next, of the cape's hem and the tail's
/// tip) and how unlike the two pictures are.
fn capes(cape: runity::netsim::NetMode, link: Conditions, seed: u64) -> ([f32; 2], [f32; 2], f32, f32) {
    let scene = runner(cape);
    let mut session = Session::new(&scene, 2, link, seed);
    for p in &mut session.peers {
        let e = p.entity(RUNNER).unwrap();
        let _ = p.world.insert_one(e, runity::netsim::Pawn);
    }
    assert!(session.join(), "{:?}", session.problems());
    session.claim(1, RUNNER);
    for t in 0..ticks(1.0) {
        session.step_with(|_, p| run(p, t));
    }
    let mut capes: Vec<Vec<Vec3>> = vec![Vec::new(); 2];
    let mut tails: Vec<Vec<Vec3>> = vec![Vec::new(); 2];
    for t in ticks(1.0)..ticks(5.0) {
        session.step_with(|_, p| run(p, t));
        for (i, p) in session.peers.iter().enumerate() {
            let (c, tail) = hanging(p);
            capes[i].push(c);
            tails[i].push(tail);
        }
    }
    assert!(session.problems().is_empty(), "{:?}", session.problems());
    // How much it shakes: the root mean square change of speed from a
    // tick to the next — the worst single one is the cape's own flapping.
    let shake = |track: &[Vec3]| {
        let n = track.len().saturating_sub(2).max(1) as f32;
        (track.windows(3).map(|w| (w[2] - 2.0 * w[1] + w[0]).length_squared()).sum::<f32>() / n).sqrt()
    };
    (
        [shake(&capes[0]), shake(&capes[1])],
        [shake(&tails[0]), shake(&tails[1])],
        unlike(&capes[0], &capes[1]),
        unlike(&tails[0], &tails[1]),
    )
}

#[test]
fn a_cape_and_a_tail_on_someone_elses_runner_do_not_shake_on_an_awful_link() {
    let (cape, tail, cape_unlike, tail_unlike) = capes(runity::netsim::NetMode::Rough, Conditions::AWFUL, 5);
    eprintln!("awful: cape shakes {cape:?}, tail {tail:?}; unlike {cape_unlike:.3} and {tail_unlike:.3}");
    // The host shows someone else's runner, a moment late and smoothed:
    // what hangs on it shakes no more than on the runner's own machine.
    assert!(cape[0] < cape[1] * 1.5 + 0.005, "the cape: {cape:?}");
    assert!(tail[0] < tail[1] * 1.5 + 0.005, "the tail: {tail:?}");
    // And looks much the same: within a hand's breadth.
    assert!(cape_unlike < 0.15, "the cape's two pictures: {cape_unlike}");
    assert!(tail_unlike < 0.2, "the tail's two pictures: {tail_unlike}");
}

#[test]
fn a_rough_cape_is_no_further_from_its_owners_than_a_local_one() {
    // The same cape Local and Rough, on an awful link, over a few seeds.
    // A flapping cape is chaos: what keeps the two pictures alike is the
    // runner shown smoothly and the wind drawn from one clock, which both
    // have. The summary may pull the host's nearer; it must not push it
    // further (docs/netsim.md: measured, within the noise either way).
    let mean = |mode| (0..3).map(|seed| capes(mode, Conditions::AWFUL, 40 + seed).2).sum::<f32>() / 3.0;
    let local = mean(runity::netsim::NetMode::Local);
    let rough = mean(runity::netsim::NetMode::Rough);
    eprintln!("cape unlike: local {local:.3}, rough {rough:.3}");
    assert!(rough < local * 1.2, "rough {rough} against local {local}");
    assert!(rough < 0.2 && local < 0.2, "both within a hand's breadth of the owner's");
}



/// The pieces a peer has of what broke, by id: where each is.
#[cfg(feature = "destruction")]
fn pieces(peer: &runity::netsim::bench::Peer) -> std::collections::BTreeMap<u64, Vec3> {
    peer.world
        .query::<(&runity::destruction::Piece, &runity::world::NetId, &Transform)>()
        .iter()
        .map(|(_, id, t)| (id.0.raw(), t.position))
        .collect()
}

#[test]
#[cfg(feature = "destruction")]
fn a_wall_broken_by_its_owner_breaks_into_the_same_pieces_everywhere_and_they_land_alike() {
    let mut session = Session::new(&wall(), 2, Conditions::POOR, 9);
    assert!(session.join(), "{:?}", session.problems());
    for _ in 0..(4.0 * HZ) as usize {
        session.step();
    }
    let host = pieces(&session.peers[0]);
    let guest = pieces(&session.peers[1]);
    assert!(host.len() >= 8, "broke: {} pieces", host.len());
    assert_eq!(host.keys().collect::<Vec<_>>(), guest.keys().collect::<Vec<_>>(), "the same pieces by the same names");
    // The host drives them; the guest shows them: where they came to rest.
    let worst = host.iter().map(|(id, at)| at.distance(guest[id])).fold(0.0f32, f32::max);
    assert!(worst < 0.05, "the pieces lie where the host has them: {worst}");
    assert_eq!(session.owners(host.keys().copied().next().unwrap()), vec![true, false]);
    assert!(session.problems().is_empty(), "{:?}", session.problems());
}



/// How high the ragdoll's pelvis is on a peer, and how hard its muscles
/// pull there.
#[cfg(feature = "character")]
fn standing(peer: &runity::netsim::bench::Peer) -> (f32, f32) {
    let e = peer.entity(PERSON).unwrap();
    let s = peer.world.get::<&runity::character::RagdollState>(e).unwrap();
    let pelvis = s.parts.first().and_then(|p| peer.world.get::<&Transform>(*p).ok().map(|t| t.position.y)).unwrap_or(f32::NAN);
    (pelvis, s.active)
}

#[test]
#[cfg(feature = "character")]
fn a_player_knocked_limp_sags_and_gets_up_alike_for_everyone() {
    let mut session = Session::new(&person(), 2, Conditions::POOR, 13);
    assert!(session.join(), "{:?}", session.problems());
    // The guest's own body.
    for p in &mut session.peers {
        let e = p.entity(PERSON).unwrap();
        let _ = p.world.insert_one(e, runity::netsim::Pawn);
    }
    session.claim(1, PERSON);
    for _ in 0..ticks(1.5) {
        session.step();
    }
    let parts: Vec<u64> = {
        let p = &session.peers[1];
        let e = p.entity(PERSON).unwrap();
        let s = p.world.get::<&runity::character::RagdollState>(e).unwrap();
        s.parts.iter().map(|part| p.world.get::<&runity::world::SceneId>(*part).unwrap().0.raw()).collect()
    };
    assert_eq!(parts.len(), 11);
    for id in &parts {
        assert_eq!(session.owners(*id), vec![false, true], "part {id} driven with the body");
    }
    let (stood, _) = standing(&session.peers[1]);
    assert!(stood > 0.8, "standing: {stood}");
    let mut tracks: Vec<Vec<Vec3>> = vec![Vec::new(); 2];
    let mut lowest = [f32::MAX; 2];
    let mut limp = [false; 2];
    for t in 0..ticks(8.0) {
        session.step_with(|i, p| {
            // Shoved over by the chest for a moment, on the machine that
            // drives it.
            if (ticks(0.17)..ticks(0.3)).contains(&t) && i == 1 {
                let e = p.entity(PERSON).unwrap();
                let parts = p.world.get::<&runity::character::RagdollState>(e).unwrap().parts.clone();
                p.physics.set_velocity(&p.world, parts[1], Vec3::new(6.0, 0.0, 0.0));
                p.physics.set_velocity(&p.world, parts[2], Vec3::new(6.0, 0.0, 0.0));
            }
        });
        for (i, p) in session.peers.iter().enumerate() {
            let (pelvis, active) = standing(p);
            tracks[i].push(Vec3::new(0.0, pelvis, 0.0));
            lowest[i] = lowest[i].min(pelvis);
            limp[i] |= active < 0.5;
        }
    }
    let ends: Vec<f32> = session.peers.iter().map(|p| standing(p).0).collect();
    eprintln!("lowest {lowest:?}, ends {ends:?}, limp {limp:?}, unlike {}", unlike(&tracks[0], &tracks[1]));
    for i in 0..2 {
        assert!(limp[i], "peer {i} saw it knocked limp");
        assert!(lowest[i] < stood - 0.2, "peer {i} saw it sag: {} from {stood}", lowest[i]);
        assert!(ends[i] > 0.8, "peer {i} saw it back up: {}", ends[i]);
    }
    assert!(unlike(&tracks[0], &tracks[1]) < 0.1, "the same fall");
    assert!(session.problems().is_empty(), "{:?}", session.problems());
}



/// Each crate shoved at the other at 9 m/s by whoever drives it, once.
fn shove(peer: &mut runity::netsim::bench::Peer) {
    for (id, v) in [(CRATE_A, 9.0), (CRATE_B, -9.0)] {
        let Some(e) = peer.entity(id) else { continue };
        if peer.world.get::<&runity::net::Owned>(e).is_ok() {
            peer.physics.set_velocity(&peer.world, e, Vec3::new(v, 0.0, 0.0));
        }
    }
}

/// The fastest either crate went after they met, on whoever drove it.
fn after_impact(session: &mut Session) -> f32 {
    let mut fastest = 0.0f32;
    for t in 0..ticks(1.5) {
        session.step_with(|_, p| if t == 0 { shove(p) });
        if t > ticks(0.4) {
            for p in &session.peers {
                for id in [CRATE_A, CRATE_B] {
                    let e = p.entity(id).unwrap();
                    if p.world.get::<&runity::net::Owned>(e).is_ok() {
                        fastest = fastest.max(p.physics.velocity(&p.world, e).unwrap_or_default().length());
                    }
                }
            }
        }
    }
    fastest
}

#[test]
fn two_crates_shoved_head_on_by_two_players_meet_on_one_machine() {
    let mut solo = Session::new(&crates(), 1, Conditions::GOOD, 1);
    let alone = after_impact(&mut solo);
    let mut session = Session::new(&crates(), 2, Conditions::POOR, 17);
    assert!(session.join(), "{:?}", session.problems());
    session.claim(1, CRATE_B);
    for _ in 0..ticks(0.67) {
        session.step();
    }
    assert_eq!(session.owners(CRATE_B), vec![false, true]);
    let together = after_impact(&mut session);
    eprintln!("after the impact: {together} m/s, alone {alone}");
    // Taken before they met: one machine solved it, as alone.
    assert_eq!(session.owners(CRATE_A), session.owners(CRATE_B), "both on one machine by the impact");
    assert!(together < alone * 1.3 + 0.5, "no one flung: {together} against {alone} alone");
    assert!(session.problems().is_empty(), "{:?}", session.problems());
}


#[test]
fn someone_joining_mid_swing_sees_the_chain_as_everyone_does() {
    let scene = pendulum(runity::netsim::NetMode::Full);
    let mut session = Session::with_late(&scene, 2, 1, Conditions::POOR, 21);
    assert!(session.join(), "{:?}", session.problems());
    // The guest takes the chain and swings it a while.
    session.claim(1, POST);
    for _ in 0..ticks(1.5) {
        session.step();
    }
    assert!(session.join_late(), "the late one in: {:?}", session.problems());
    let mut tracks: Vec<Vec<Vec3>> = vec![Vec::new(); 3];
    for _ in 0..ticks(2.0) {
        session.step();
        for (i, s) in seen(&session).iter().enumerate() {
            tracks[i].push(s.points.get(s.points.len() / 2).copied().unwrap_or(Vec3::NAN));
        }
    }
    // The newcomer's chain swings as the owner's does, a moment behind.
    let off = unlike(&tracks[2], &tracks[1]);
    eprintln!("the late one's chain {off:.3} from the owner's");
    assert!(off < 0.05, "the late one's chain: {off}");
    assert_eq!(session.owners(POST), vec![false, true, false]);
    assert!(session.problems().is_empty(), "{:?}", session.problems());
}

#[test]
fn a_tug_of_war_keeps_within_the_budget() {
    // The two bodies and the rope, sixty times a second, sent thirty.
    let scene = tug(runity::netsim::NetMode::Full);
    let mut session = Session::new(&scene, 2, Conditions::POOR, 3);
    for p in &mut session.peers {
        for id in [PAWN_A, PAWN_B] {
            let e = p.entity(id).unwrap();
            let _ = p.world.insert_one(e, runity::netsim::Pawn);
        }
    }
    assert!(session.join(), "{:?}", session.problems());
    session.claim(1, PAWN_B);
    for _ in 0..ticks(0.5) {
        session.step();
    }
    let (sent, served) = (session.sent.load(std::sync::atomic::Ordering::Relaxed), session.served.load(std::sync::atomic::Ordering::Relaxed));
    let seconds = 3.0;
    for _ in 0..ticks(seconds) {
        session.step_with(|_, p| pull(p, 200.0, 700.0));
    }
    let up = (session.sent.load(std::sync::atomic::Ordering::Relaxed) - sent) as f32 / seconds;
    let down = (session.served.load(std::sync::atomic::Ordering::Relaxed) - served) as f32 / seconds;
    eprintln!("the guest sends {:.1} KB/s and is sent {:.1} KB/s", up / 1024.0, down / 1024.0);
    assert!(up < 30.0 * 1024.0 && down < 30.0 * 1024.0, "{up} up, {down} down, bytes a second");
}

#[test]
fn ten_chains_swinging_at_once_keep_within_the_budget_and_none_is_left_behind() {
    // Ten chains with loads, all the guest's: more changes a tick than
    // the budget carries. The most waited go first, so each goes often
    // enough, and the link stays within 30 KB a second.
    let one = pendulum(runity::netsim::NetMode::Full);
    let mut scene = Scene::default();
    scene.entities.push(one.entities[0].clone());
    for k in 0..10u64 {
        let mut post = one.entities[1].clone();
        let mut load = one.entities[2].clone();
        post.id = EntityId::from_raw(100 + k * 2);
        load.id = EntityId::from_raw(101 + k * 2);
        post.transform.position.z = k as f32 * 1.5;
        load.transform.position.z = k as f32 * 1.5;
        post.set_part(&Rope {
            to: Vec3::ZERO,
            end: EntityRef::to(load.id),
            kind: RopeKind::Chain,
            slack: 0.0,
            segments: 16,
            thickness: 0.03,
            net: runity::netsim::NetMode::Full,
            ..Default::default()
        });
        scene.entities.push(post);
        scene.entities.push(load);
    }
    let mut session = Session::new(&scene, 2, Conditions::POOR, 5);
    assert!(session.join(), "{:?}", session.problems());
    for k in 0..10u64 {
        session.claim(1, 100 + k * 2);
    }
    for _ in 0..ticks(0.5) {
        session.step();
    }
    let sent = session.sent.load(std::sync::atomic::Ordering::Relaxed);
    let seconds = 3.0;
    let mut tracks: Vec<[Vec<Vec3>; 2]> = (0..10).map(|_| [Vec::new(), Vec::new()]).collect();
    for _ in 0..ticks(seconds) {
        session.step();
        for k in 0..10 {
            for (i, p) in session.peers.iter().enumerate() {
                let e = p.entity(101 + k as u64 * 2).unwrap();
                tracks[k][i].push(p.world.get::<&Transform>(e).unwrap().position);
            }
        }
    }
    let up = (session.sent.load(std::sync::atomic::Ordering::Relaxed) - sent) as f32 / seconds;
    // Shown later than one chain alone would be — each goes less often,
    // and the buffer waits longer — but shown right.
    let worst = tracks.iter().map(|t| unlike_within(&t[0], &t[1], 20)).fold(0.0f32, f32::max);
    eprintln!("ten chains: the guest sends {:.1} KB/s; the host's loads at worst {worst:.3} m from the guest's", up / 1024.0);
    assert!(up < 31.0 * 1024.0, "{up} bytes a second");
    assert!(worst < 0.15, "every load follows: {worst}");
}

/// What each scene costs on the wire, a table: the guest's upload and
/// download, KB a second, over three seconds of it playing. Ignored: run
/// it to see what an optimisation of the wire saved.
#[test]
#[ignore]
fn traffic() {
    use std::sync::atomic::Ordering::Relaxed;
    fn measure(name: &str, scene: Scene, pawns: &[u64], claims: &[u64], game: impl FnMut(usize, usize, &mut runity::netsim::bench::Peer)) {
        measure_tagged(name, scene, pawns, claims, None, game)
    }
    /// A game's own networked component, as a player's might be: a name
    /// and health that seldom change.
    #[derive(Clone, serde::Serialize, serde::Deserialize)]
    struct Tag {
        name: String,
        health: u32,
    }
    fn measure_tagged(
        name: &str,
        scene: Scene,
        pawns: &[u64],
        claims: &[u64],
        tag: Option<u64>,
        mut game: impl FnMut(usize, usize, &mut runity::netsim::bench::Peer),
    ) {
        let mut session = Session::new(&scene, 2, Conditions::GOOD, 1);
        if let Some(id) = tag {
            session.components.register_networked::<Tag>("tag");
            for p in &mut session.peers {
                if let Some(e) = p.entity(id) {
                    let _ = p.world.insert_one(e, Tag { name: "Valentina the Unready".into(), health: 100 });
                }
            }
        }
        for p in &mut session.peers {
            for id in pawns {
                if let Some(e) = p.entity(*id) {
                    let _ = p.world.insert_one(e, runity::netsim::Pawn);
                }
            }
        }
        assert!(session.join());
        for id in claims {
            session.claim(1, *id);
        }
        for t in 0..ticks(0.5) {
            session.step_with(|i, p| game(t, i, p));
        }
        let (up, down) = (session.sent.load(Relaxed), session.served.load(Relaxed));
        let (up_kinds, down_kinds) = (session.sent_kinds.read(), session.served_kinds.read());
        for kinds in [&session.sent_kinds, &session.served_kinds] {
            *kinds.kept.lock().unwrap() = Some(Vec::new());
        }
        let seconds = 3.0;
        for t in ticks(0.5)..ticks(0.5 + seconds) {
            session.step_with(|i, p| game(t, i, p));
        }
        let up = (session.sent.load(Relaxed) - up) as f32 / seconds / 1024.0;
        let down = (session.served.load(Relaxed) - down) as f32 / seconds / 1024.0;
        let by = |now: [(u64, u64); 8], was: [(u64, u64); 8]| {
            ["unrel", "rel", "ack", "ping", "bye"]
                .iter()
                .enumerate()
                .map(|(i, k)| format!("{k} {:.0}B/{:.0}", (now[i].0 - was[i].0) as f32 / seconds, (now[i].1 - was[i].1) as f32 / seconds))
                .collect::<Vec<_>>()
                .join(" ")
        };
        println!("TRAFFIC {name:<14} guest up {up:6.2} KB/s  down {down:6.2} KB/s");
        println!("   up:   {}", by(session.sent_kinds.read(), up_kinds));
        println!("   down: {}", by(session.served_kinds.read(), down_kinds));
        // Would deflating each datagram pay? (Each alone: an unreliable
        // one cannot lean on the one before, which may not have come.)
        for (way, kinds) in [("up", &session.sent_kinds), ("down", &session.served_kinds)] {
            let kept = kinds.kept.lock().unwrap().take().unwrap_or_default();
            let raw: usize = kept.iter().map(Vec::len).sum();
            let deflated: usize = kept.iter().map(|d| miniz_oxide::deflate::compress_to_vec(d, 9).len().min(d.len()) + 1).sum();
            if raw > 0 {
                println!("   {way} deflated: {:.0}% of {raw} bytes", deflated as f32 * 100.0 / raw as f32);
            }
        }
    }
    measure("idle crates", crates(), &[], &[CRATE_B], |_, _, _| {});
    measure("crates", crates(), &[], &[CRATE_B], |t, _, p| {
        if t % ticks(1.0) == 0 {
            for (id, v) in [(CRATE_A, 4.0), (CRATE_B, -4.0)] {
                if let Some(e) = p.entity(id) {
                    if p.world.get::<&runity::net::Owned>(e).is_ok() {
                        p.physics.set_velocity(&p.world, e, Vec3::new(v, 3.0, 0.0));
                    }
                }
            }
        }
    });
    measure("chain", pendulum(runity::netsim::NetMode::Full), &[], &[POST], |_, _, _| {});
    measure("tug", tug(runity::netsim::NetMode::Full), &[PAWN_A, PAWN_B], &[PAWN_B], |_, _, p| {
        for (id, f) in [(PAWN_A, -200.0), (PAWN_B, 700.0)] {
            if let Some(e) = p.entity(id) {
                if p.world.get::<&runity::net::Owned>(e).is_ok() {
                    p.physics.add_force(&p.world, e, Vec3::new(f, 0.0, 0.0));
                }
            }
        }
    });
    let run = |t: usize, _: usize, p: &mut runity::netsim::bench::Peer| {
        let Some(e) = p.entity(RUNNER) else { return };
        if p.world.get::<&runity::net::Owned>(e).is_err() {
            return;
        }
        let angle = t as f32 / HZ;
        if let Ok(mut tr) = p.world.get::<&mut Transform>(e) {
            tr.position = Vec3::new(angle.cos() * 3.0, 0.8, angle.sin() * 3.0);
            tr.set_rotation(runity::glam::Quat::from_rotation_y(-angle));
        }
    };
    measure("runner", runner(runity::netsim::NetMode::Rough), &[RUNNER], &[RUNNER], run);
    measure_tagged("tagged runner", runner(runity::netsim::NetMode::Rough), &[RUNNER], &[RUNNER], Some(RUNNER), run);
    measure("ragdoll", person(), &[PERSON], &[PERSON], |_, _, _| {});
}
