//! Twelve settlers, three sim-days, and the one stake the settlement decides
//! to plant.
//!
//! The unit tests beside [`runity_core::mind::SettlementMind`] hand the
//! planner a deficit log filled in by hand. This one never touches the log:
//! it runs [`runity_core::needs::tick_settlement`] over a real world for
//! three days and lets the nights write themselves, so what is being checked
//! is the whole path — a night lived through, a reading, a ranking, a cell, a
//! planter, a stake in the world — rather than any one link of it.
//!
//! Nothing here asserts a tuned number. What it asserts is the shape: that
//! nothing is staked before anything has happened, that exactly one thing is
//! staked afterwards, that which thing it is moves when the settlement's
//! nights move, and that the run reproduces itself exactly.

use runity_core::economy::{DeficitKind, Stake, StrikeBoard};
use runity_core::mind::{Plan, SettlementMind, Survey, Wake};
use runity_core::needs::{tick_settlement, DeficitLog, Hearth, Hurt, NeedKind, Needs, NightWatch};
use runity_core::physics::Heightfield;
use runity_core::placement::{GridCell, PlacementGrid, PlacementWeights, CELL_SIZE, GRID_SIDE};
use runity_core::world::Entity;
use runity_core::{Transform, World};
use runity_math::{Vec2, Vec3};

/// Twenty-four ticks to the sim-hour at a 20 Hz step, the same sim-day
/// `mind_settlement.rs` runs on.
const TICKS_PER_DAY: u64 = 480;
const SECONDS_PER_TICK: f32 = 1.0 / 20.0;
const DAYS: u64 = 3;
const SETTLERS: usize = 12;

/// One heightfield node per placement-grid cell corner.
const GROUND_SIDE: usize = GRID_SIDE + 1;

fn flat_ground() -> Heightfield {
    Heightfield::new(
        Vec2::ZERO,
        CELL_SIZE,
        GROUND_SIDE,
        GROUND_SIDE,
        vec![0.0; GROUND_SIDE * GROUND_SIDE],
    )
}

/// The camp of `12-minds.md` part 0: twelve settlers in a ring, each of them
/// keeping their own tally of what the nights do to them.
fn camp() -> (World, Vec<Entity>) {
    let mut world = World::new();
    let settlers = (0..SETTLERS)
        .map(|i| {
            // A ragged ring near the origin corner of the grid, laid out
            // without trigonometry so the arrangement is a fact of the test
            // and not of anyone's `sin`.
            let position = Vec3::new(
                120.0 + (i % 4) as f32 * 9.0,
                0.0,
                120.0 + (i / 4) as f32 * 11.0,
            );
            let settler = world.spawn();
            world.insert(settler, Transform::from_position(position));
            world.insert(settler, Needs::default());
            world.insert(settler, NightWatch::default());
            world.insert(settler, Hurt::new());
            settler
        })
        .collect();
    (world, settlers)
}

fn light_a_fire(world: &mut World, position: Vec3, radius: f32) {
    let hearth = world.spawn();
    world.insert(hearth, Transform::from_position(position));
    world.insert(hearth, Hearth::new(radius));
}

/// One stake as the run saw it being planted.
#[derive(Debug, Clone, PartialEq)]
struct Planted {
    tick: u64,
    kind: DeficitKind,
    cell: GridCell,
    planter: Entity,
    site: Vec2,
}

impl Planted {
    fn of(tick: u64, plan: &Plan) -> Self {
        Self {
            tick,
            kind: plan.kind(),
            cell: plan.cell(),
            planter: plan.planter(),
            site: plan.site(),
        }
    }
}

/// Run `world` for [`DAYS`] sim-days with the settlement mind wired to the
/// two things that can wake it here: the periodic sweep and the night log
/// closing at dawn. Returns everything that got planted, and the world it
/// got planted in.
fn run(mut world: World) -> (World, Vec<Planted>) {
    let ground = flat_ground();
    let mut log = DeficitLog::new(TICKS_PER_DAY);
    let strikes = StrikeBoard::new();
    let mut mind = SettlementMind::new(TICKS_PER_DAY);
    let mut grid = PlacementGrid::new(Vec2::ZERO, PlacementWeights::default());
    grid.recompute(&world, &ground);

    let mut stake_entity: Option<Entity> = None;
    let mut planted = Vec::new();

    for tick in 0..=(DAYS * TICKS_PER_DAY) {
        // Whether this tick is the one that closes the night has to be asked
        // before the bookkeeping runs, because running it is what closes it.
        let dawn = log.dawn_due(tick);
        tick_settlement(&mut world, &mut log, tick, SECONDS_PER_TICK);

        let plan = {
            let survey = Survey {
                tick,
                world: &world,
                log: &log,
                grid: &grid,
                strikes: &strikes,
                stake: stake_entity.and_then(|entity| world.get::<Stake>(entity)),
            };
            if dawn {
                mind.wake(Wake::NightLogClosed, &survey)
            } else {
                mind.update(&survey)
            }
        };

        if let Some(plan) = plan {
            planted.push(Planted::of(tick, &plan));
            stake_entity = Some(plan.plant(&mut world, &ground));
            // A stake claims the ground it stands on.
            grid.mark_dirty();
            grid.recompute(&world, &ground);
        }
    }

    (world, planted)
}

#[test]
fn a_camp_with_no_fire_and_no_roof_stakes_the_hearth_first_and_only_once() {
    let (world, settlers) = camp();
    let (world, planted) = run(world);

    assert_eq!(
        planted.len(),
        1,
        "a settlement has one assignment at a time: {planted:?}"
    );
    let first = &planted[0];

    assert_eq!(
        first.tick, TICKS_PER_DAY,
        "the first night is the first thing that has ever happened, and the \
         planner acts on the dawn that closes it rather than waiting out the \
         sweep"
    );
    assert_eq!(
        first.kind,
        DeficitKind::Warmth,
        "everyone was equally cold and equally roofless, and a hearth is the \
         cheaper relief per unit carried"
    );
    assert!(settlers.contains(&first.planter));

    // The stake is in the world, at the site, and asking for something.
    let (stake_entity, stake) = world
        .iter::<Stake>()
        .map(|(e, s)| (e, s.clone()))
        .next()
        .expect("the plan was planted");
    assert!(
        !settlers.contains(&stake_entity),
        "on ground, not on a person"
    );
    assert_eq!(stake.kind(), DeficitKind::Warmth);
    assert!(stake.target_units() > 0);
    assert_eq!(stake.total_units(), 0, "the planner carries nothing itself");
    let at = world.get::<Transform>(stake_entity).unwrap().position;
    assert_eq!((at.x, at.z), (first.site.x, first.site.y));
}

#[test]
fn nothing_is_staked_until_a_night_has_actually_been_lived_through() {
    let (world, _) = camp();
    let (_, planted) = run(world);
    assert!(
        planted.iter().all(|p| p.tick >= TICKS_PER_DAY),
        "the planner read something before anything happened: {planted:?}"
    );
}

#[test]
fn a_fire_big_enough_for_everyone_moves_the_stake_to_the_hut() {
    let (mut world, _) = camp();
    // One hearth covering the whole camp: warmth reads 0.0 at dawn, which is
    // definable and worthless, while shelter still reads 1.0.
    light_a_fire(&mut world, Vec3::new(135.0, 0.0, 135.0), 60.0);

    let (_, planted) = run(world);

    assert_eq!(planted.len(), 1);
    assert_eq!(
        planted[0].kind,
        DeficitKind::Shelter,
        "a hearth is six times cheaper and would win on any night the camp \
         went cold, but a night nobody went cold is worth nothing per unit \
         carried: {planted:?}"
    );
}

#[test]
fn the_settler_sent_is_the_nearest_of_those_the_night_hit_hardest() {
    let (mut world, settlers) = camp();
    // A small fire that reaches exactly one settler, so after the first
    // night eleven of the twelve carry a warmth count and one does not.
    let sheltered = settlers[0];
    let by_the_fire = world.get::<Transform>(sheltered).unwrap().position;
    light_a_fire(&mut world, by_the_fire, 1.0);

    let (world, planted) = run(world);
    let first = &planted[0];
    assert_eq!(first.kind, DeficitKind::Warmth);
    assert_ne!(
        first.planter, sheltered,
        "the one person this never happened to is the one person not sent"
    );

    // The rule, restated from the far side: nobody was hit harder, and
    // nobody hit as hard stood closer.
    let hurt_of = |entity: Entity| {
        world
            .get::<Hurt>(entity)
            .unwrap()
            .count(DeficitKind::Warmth)
    };
    let distance_of = |entity: Entity| {
        let p = world.get::<Transform>(entity).unwrap().position;
        (Vec2::new(p.x, p.z) - first.site).length()
    };
    let chosen_hurt = hurt_of(first.planter);
    let chosen_distance = distance_of(first.planter);
    assert!(chosen_hurt > 0);
    for &settler in &settlers {
        assert!(
            hurt_of(settler) <= chosen_hurt,
            "someone the cold hit harder was passed over"
        );
        if hurt_of(settler) == chosen_hurt {
            assert!(
                distance_of(settler) >= chosen_distance,
                "someone equally cold was standing closer"
            );
        }
    }
}

#[test]
fn the_same_three_days_decide_the_same_way_twice() {
    let (first_world, _) = camp();
    let (second_world, _) = camp();
    let (_, first) = run(first_world);
    let (_, second) = run(second_world);
    assert_eq!(
        first, second,
        "there is no randomness in either mind: the sweep is a counter and \
         the ranking is a sort"
    );
}

#[test]
fn the_planner_leaves_the_settlers_needs_entirely_alone() {
    let (world, settlers) = camp();

    // What three days of `tick_settlement` alone do to everyone.
    let mut untouched = {
        let (world, _) = camp();
        world
    };
    let mut log = DeficitLog::new(TICKS_PER_DAY);
    for tick in 0..=(DAYS * TICKS_PER_DAY) {
        tick_settlement(&mut untouched, &mut log, tick, SECONDS_PER_TICK);
    }

    let (planned, planted) = run(world);
    assert_eq!(planted.len(), 1, "the planner did in fact do something");

    for &settler in &settlers {
        let before = untouched.get::<Needs>(settler).unwrap();
        let after = planned.get::<Needs>(settler).unwrap();
        for kind in [NeedKind::Hunger, NeedKind::Warmth, NeedKind::Rest] {
            assert_eq!(
                before.level(kind),
                after.level(kind),
                "the planner moved a settler's {kind:?}"
            );
        }
        assert_eq!(
            untouched.get::<Transform>(settler).unwrap().position,
            planned.get::<Transform>(settler).unwrap().position,
            "the planner moved a settler"
        );
    }
}
