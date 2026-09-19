//! [`SettlementMind`]: one stake, chosen by severity ÷ cost, staked by
//! whoever hurt most.

use runity_math::{Vec2, Vec3};

use crate::economy::{DeficitKind, Stake, StrikeBoard, Tag};
use crate::needs::{DeficitLog, Hurt};
use crate::physics::Heightfield;
use crate::placement::{GridCell, PlacementGrid};
use crate::transform::Transform;
use crate::world::{Entity, World};

/// What one of the four stakes asks for: a property and an amount, never a
/// specific material (`05-economy.md` §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildCost {
    /// The property the stake asks for.
    pub tag: Tag,
    /// How many units of it the building takes.
    pub units: u32,
}

impl BuildCost {
    /// `units` units of `tag`.
    pub const fn new(tag: Tag, units: u32) -> Self {
        Self { tag, units }
    }
}

/// Every number the settlement mind reads, in one struct, in the spirit of
/// [`super::MindTuning`] and [`crate::placement::PlacementWeights`].
///
/// The four costs are the divisor in severity ÷ cost, so they are not
/// decoration: making a hearth six times cheaper than a hut is the whole
/// reason a settlement that is equally cold and equally roofless builds the
/// hearth first. Only one of them comes from the design set — a storehouse is
/// the "hard, 30 units" barn of `05-economy.md` §3.2 — and the other three are
/// placeholders scaled around it until playtesting says otherwise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlannerTuning {
    /// How many ticks apart the periodic wakes are. A sweep falls on every
    /// tick where `tick % period_ticks == 0`: an absolute grid, like the
    /// personal mind's [`super::staggered_recompute`], so a replay wakes the
    /// planner on exactly the same ticks (`08-scale.md`). Zero means every
    /// tick.
    pub period_ticks: u64,
    /// Severity a deficit has to reach before it is worth a stake at all.
    ///
    /// `12-minds.md` part 5 leaves this threshold unformalised on purpose —
    /// it is calibrated against `11-verification.md`, not argued from the
    /// design — so the default is zero: every *definable* deficit competes,
    /// and undefined ones are excluded by being undefined rather than by
    /// being small.
    pub severity_threshold: f32,
    /// Изба — the first hut.
    pub hut: BuildCost,
    /// Очаг — any hearth beyond the starting one.
    pub hearth: BuildCost,
    /// Хранилище — the first storehouse, the barn of `05-economy.md` §3.2.
    pub storehouse: BuildCost,
    /// Сушило — the first drying rack.
    pub drying_rack: BuildCost,
}

impl Default for PlannerTuning {
    fn default() -> Self {
        Self {
            period_ticks: 900,
            severity_threshold: 0.0,
            hut: BuildCost::new(Tag::Hard, 60),
            hearth: BuildCost::new(Tag::Hard, 10),
            storehouse: BuildCost::new(Tag::Hard, 30),
            drying_rack: BuildCost::new(Tag::Fibrous, 12),
        }
    }
}

impl PlannerTuning {
    /// What a stake for `kind` asks for.
    pub fn cost_of(&self, kind: DeficitKind) -> BuildCost {
        match kind {
            DeficitKind::Shelter => self.hut,
            DeficitKind::Warmth => self.hearth,
            DeficitKind::Storage => self.storehouse,
            DeficitKind::Drying => self.drying_rack,
        }
    }
}

/// Why the settlement mind is thinking on this tick.
///
/// Four of the five are events rather than clock: a settlement that has just
/// lost its hearth should not have to wait out the rest of the sweep to
/// notice. The fifth is the sweep itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// The every-[`PlannerTuning::period_ticks`] sweep came due.
    Period,
    /// The settlement's stake was finished, so its one pair of hands is free.
    StakeFinished,
    /// A hearth went out.
    HearthDied,
    /// A settler died.
    SettlerDied,
    /// Dawn closed the night log, and the two night-bound readings are new.
    NightLogClosed,
}

/// One deficit as the planner ranks it: how badly it is being lived through,
/// what treating it costs, and the quotient that decides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    /// Which deficit.
    pub kind: DeficitKind,
    /// Its severity right now, from [`severities`].
    pub severity: f32,
    /// Units of property the stake for it would ask for.
    pub cost: u32,
    /// `severity / cost`. Not severity alone: the settlement has one pair of
    /// hands and finite material, so what it is actually choosing between is
    /// relief per unit carried, not raw pain.
    pub score: f32,
}

/// Everything the settlement mind is allowed to look at, gathered for one
/// think.
///
/// Every field is a shared reference. That is the point, and it is check #8
/// of `12-minds.md`: the planner reads the settlement and writes exactly one
/// thing — a stake — and there is no route from anything in here to a settler
/// it could mutate.
pub struct Survey<'a> {
    /// The tick being thought on.
    pub tick: u64,
    /// Where the settlers, hearths and stakes are.
    pub world: &'a World,
    /// The four readings, as lived through.
    pub log: &'a DeficitLog,
    /// Where a stake of a given kind would go. Must have been recomputed at
    /// least once, or it offers no cell and the planner opens nothing.
    pub grid: &'a PlacementGrid,
    /// Which deficits have already burned through three attempts.
    pub strikes: &'a StrikeBoard,
    /// The settlement's stake, if it has one. A settlement has at most one,
    /// and while it stands — active or merely frozen — the planner opens
    /// nothing else.
    pub stake: Option<&'a Stake>,
}

/// What the settlement mind decided: one stake, the cell it goes in, and the
/// one settler being sent to plant it.
///
/// A plan is a value, not an act. Holding one grants nothing: the settler
/// appears as an [`Entity`], which is a generational handle with no methods
/// beyond its own two numbers, and the stake is not in the world until
/// [`Plan::plant`] puts it there.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    kind: DeficitKind,
    cell: GridCell,
    site: Vec2,
    planter: Entity,
    stake: Stake,
}

impl Plan {
    /// Which deficit this stake answers.
    pub fn kind(&self) -> DeficitKind {
        self.kind
    }

    /// The cell [`PlacementGrid::best_cell_for`] picked.
    pub fn cell(&self) -> GridCell {
        self.cell
    }

    /// World XZ of the middle of that cell.
    pub fn site(&self) -> Vec2 {
        self.site
    }

    /// The settler being sent to plant it: nearest to the site among those
    /// this exact deficit hit hardest personally (`12-minds.md` §2.4).
    ///
    /// Sending is all it is. The planner does not assign them to build, does
    /// not hold them there, and cannot reach their needs to make them want
    /// to — part 5 is emphatic that a stake is a fact in the world and not an
    /// order, and this handle is not enough to make it one.
    pub fn planter(&self) -> Entity {
        self.planter
    }

    /// The stake itself, before it is in the world.
    pub fn stake(&self) -> &Stake {
        &self.stake
    }

    /// Put the stake in the world: a fresh entity carrying it and a
    /// [`Transform`] at the site, standing on `ground`.
    ///
    /// This spawns and writes; it never reads or modifies an entity that
    /// already exists, least of all the planter's. That the planner's one
    /// effect on the world is an insert on a brand-new entity is what check
    /// #8 is checking.
    pub fn plant(self, world: &mut World, ground: &Heightfield) -> Entity {
        let entity = world.spawn();
        let height = ground.height_at(self.site.x, self.site.y);
        world.insert(
            entity,
            Transform::from_position(Vec3::new(self.site.x, height, self.site.y)),
        );
        world.insert(entity, self.stake);
        entity
    }
}

/// The four readings [`severities`] takes off a [`DeficitLog`], in
/// [`DeficitKind::ALL`]'s order, each one either a severity or undefined.
pub type Severities = [(DeficitKind, Option<f32>); 4];

/// How severely each of the four deficits is being lived through, in
/// [`DeficitKind::ALL`]'s order — `None` for one still undefined.
///
/// The `Option` is the whole reason [`DeficitLog`] reads in options: a
/// deficit whose event has never happened is not a deficit of zero, and zero
/// is the *best* possible severity, so defaulting would quietly make an
/// undefined deficit the least urgent thing rather than no thing at all.
///
/// The pairing is the log's four entries against the building that answers
/// each, per part 3's table: a settler who slept off every sleeping spot
/// wanted a hut, one who slept away from every hearth wanted a hearth, a
/// marked item that aged into forgetting wanted somewhere to put it, and an
/// attempt to eat that found nothing wanted the food a drying rack keeps
/// through the winter.
pub fn severities(log: &DeficitLog) -> Severities {
    [
        (DeficitKind::Shelter, log.unsheltered_last_night()),
        (DeficitKind::Warmth, log.cold_last_night()),
        (DeficitKind::Storage, log.forgotten_items_deficit()),
        (DeficitKind::Drying, log.hunger_deficit()),
    ]
}

/// The settler `12-minds.md` §2.4 sends to plant a stake for `kind` at
/// `site`: nearest to the site among those this exact deficit hit hardest
/// personally. `None` if nobody in `world` carries a [`Hurt`] tally at all.
///
/// The two halves are lexicographic and in that order, which is what makes
/// the rule hold by construction rather than by tuning: hurt is compared
/// first and distance only breaks ties within it, so no amount of standing
/// closer promotes a settler over one the deficit hit harder. Entity order
/// breaks the last tie, so the choice does not depend on which order the
/// world happens to store its components in.
///
/// A deficit that has hit nobody personally — the settlement's reading says
/// it happened, but no settler recorded it — leaves everyone tied at zero,
/// and the rule degenerates to "the nearest". That is the honest reading of
/// "hit hardest" over an all-zero tally, and it keeps the planner total: it
/// always has someone to send if anyone is alive.
pub fn planter_for(world: &World, kind: DeficitKind, site: Vec2) -> Option<Entity> {
    world
        .iter::<Hurt>()
        .map(|(entity, hurt)| {
            let position = world
                .get::<Transform>(entity)
                .map(|t| t.position)
                .unwrap_or(Vec3::ZERO);
            let distance = (Vec2::new(position.x, position.z) - site).length();
            (entity, hurt.count(kind), distance)
        })
        .min_by(|a, b| b.1.cmp(&a.1).then(a.2.total_cmp(&b.2)).then(a.0.cmp(&b.0)))
        .map(|(entity, _, _)| entity)
}

/// The second mind of `12-minds.md`: not an NPC, not an elder, not a task
/// dispatcher — the part of the simulation that looks at the settlement whole
/// and, now and then, plants one stake.
///
/// It wakes on the [`PlannerTuning::period_ticks`] sweep
/// ([`SettlementMind::update`]) and on four events
/// ([`SettlementMind::wake`]), reads the deficit log, throws out every
/// deficit still undefined, scores the rest by severity ÷ build cost, and
/// opens the best of them as the settlement's one stake at the best cell the
/// [`PlacementGrid`] offers — but only if the settlement has no stake
/// standing and the deficit has not already burned through three attempts.
///
/// What it cannot do is the shape of the type. Its public API takes no
/// mutable reference to anything a settler is made of: a [`Survey`] is all
/// shared references, a [`Plan`] names its planter as a bare [`Entity`], and
/// [`Plan::plant`] spawns. `12-minds.md`'s verification check #8 is therefore
/// not a convention anybody has to keep — there is no signature in here that
/// could be used to break it.
///
/// ```
/// use runity_core::economy::{DeficitKind, StrikeBoard};
/// use runity_core::mind::{SettlementMind, Survey, Wake};
/// use runity_core::needs::{DeficitLog, Hurt};
/// use runity_core::physics::Heightfield;
/// use runity_core::placement::{PlacementGrid, PlacementWeights};
/// use runity_core::{Transform, World};
/// use runity_math::{Vec2, Vec3};
///
/// let ticks_per_day = 480;
/// let mut world = World::new();
/// let settler = world.spawn();
/// world.insert(settler, Transform::from_position(Vec3::new(40.0, 0.0, 40.0)));
/// world.insert(settler, Hurt::new());
///
/// let ground = Heightfield::new(Vec2::ZERO, 8.0, 65, 65, vec![0.0; 65 * 65]);
/// let mut grid = PlacementGrid::new(Vec2::ZERO, PlacementWeights::default());
/// grid.recompute(&world, &ground);
///
/// let mut log = DeficitLog::new(ticks_per_day);
/// let strikes = StrikeBoard::new();
/// let mut mind = SettlementMind::new(ticks_per_day);
///
/// // Tick zero: nothing has been lived through, so there is nothing to
/// // weigh and nothing is staked.
/// assert!(mind
///     .update(&Survey {
///         tick: 0,
///         world: &world,
///         log: &log,
///         grid: &grid,
///         strikes: &strikes,
///         stake: None,
///     })
///     .is_none());
///
/// // A night passes with nobody by a fire, and dawn closes the log.
/// log.record_night(1.0, 0.0, ticks_per_day);
/// let plan = mind
///     .wake(
///         Wake::NightLogClosed,
///         &Survey {
///             tick: ticks_per_day,
///             world: &world,
///             log: &log,
///             grid: &grid,
///             strikes: &strikes,
///             stake: None,
///         },
///     )
///     .expect("a whole village went cold");
/// assert_eq!(plan.kind(), DeficitKind::Warmth);
/// assert_eq!(plan.planter(), settler, "the only person it happened to");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct SettlementMind {
    ticks_per_day: u64,
    tuning: PlannerTuning,
    last_wake: Option<(u64, Wake)>,
}

impl SettlementMind {
    /// A planner for a settlement whose sim-day is `ticks_per_day` fixed
    /// steps long, on the default tuning.
    pub fn new(ticks_per_day: u64) -> Self {
        Self::with_tuning(ticks_per_day, PlannerTuning::default())
    }

    /// The same, on a tuning of your own.
    pub fn with_tuning(ticks_per_day: u64, tuning: PlannerTuning) -> Self {
        assert!(
            ticks_per_day > 0,
            "a sim-day must be at least one tick long"
        );
        Self {
            ticks_per_day,
            tuning,
            last_wake: None,
        }
    }

    /// The numbers this planner is weighing with.
    pub fn tuning(&self) -> &PlannerTuning {
        &self.tuning
    }

    /// When it last thought, and what woke it — for anyone watching a run
    /// rather than a test.
    pub fn last_wake(&self) -> Option<(u64, Wake)> {
        self.last_wake
    }

    /// Whether the periodic sweep falls on `tick`.
    pub fn sweep_due(&self, tick: u64) -> bool {
        self.tuning.period_ticks == 0 || tick % self.tuning.period_ticks == 0
    }

    /// One tick of the settlement mind: think if the sweep is due, and
    /// otherwise do nothing at all. Cheap to call every tick for that reason.
    pub fn update(&mut self, survey: &Survey<'_>) -> Option<Plan> {
        if !self.sweep_due(survey.tick) {
            return None;
        }
        self.wake(Wake::Period, survey)
    }

    /// Think now, because `event` happened, without waiting for the sweep.
    ///
    /// Thinking twice on one tick is a no-op: the second call would read the
    /// identical facts and hand back a second copy of the same stake, and a
    /// settlement has one. Whichever wake came first on that tick is the one
    /// that counts.
    pub fn wake(&mut self, event: Wake, survey: &Survey<'_>) -> Option<Plan> {
        if self.last_wake.is_some_and(|(tick, _)| tick == survey.tick) {
            return None;
        }
        self.last_wake = Some((survey.tick, event));
        self.consider(survey)
    }

    /// Every deficit worth treating, best first: definable, above
    /// [`PlannerTuning::severity_threshold`], not on a three-strikes hold,
    /// sorted by severity ÷ cost.
    ///
    /// Sorting is stable and the input is [`DeficitKind::ALL`], so two
    /// deficits that score exactly alike always rank in that order — the same
    /// facts give the same stake on every machine and every replay.
    pub fn ranking(&self, survey: &Survey<'_>) -> Vec<Candidate> {
        let mut ranked: Vec<Candidate> = severities(survey.log)
            .into_iter()
            .filter_map(|(kind, severity)| {
                // Undefined is not zero. A deficit whose event has never
                // happened is not in the running at all.
                let severity = severity?;
                if severity < self.tuning.severity_threshold {
                    return None;
                }
                if !survey.strikes.is_treatable(kind) {
                    return None;
                }
                let cost = self.tuning.cost_of(kind);
                Some(Candidate {
                    kind,
                    severity,
                    cost: cost.units,
                    score: severity / cost.units as f32,
                })
            })
            .collect();
        ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
        ranked
    }

    /// The whole decision, without the wake bookkeeping: rank, pick the best,
    /// find it ground and someone to plant it.
    fn consider(&self, survey: &Survey<'_>) -> Option<Plan> {
        // One stake at a time. Frozen counts as standing: it is paused, not
        // gone, and the settlement's one assignment is still spoken for.
        if survey.stake.is_some_and(|stake| !stake.is_abandoned()) {
            return None;
        }

        let best = self.ranking(survey).into_iter().next()?;
        let cost = self.tuning.cost_of(best.kind);
        let cell = survey.grid.best_cell_for(best.kind)?;
        let site = survey.grid.cell_center(cell);
        let planter = planter_for(survey.world, best.kind, site)?;
        // Through the board rather than straight to `Stake::new`: the
        // three-strikes rule belongs to the board, and asking it twice is
        // cheaper than letting the two copies drift apart.
        let stake = survey
            .strikes
            .open_stake(
                best.kind,
                cost.tag,
                cost.units,
                self.ticks_per_day,
                survey.tick,
            )
            .ok()?;

        Some(Plan {
            kind: best.kind,
            cell,
            site,
            planter,
            stake,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placement::{PlacementWeights, CELL_SIZE, GRID_SIDE};

    const TICKS_PER_DAY: u64 = 480;
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

    fn settler_at(world: &mut World, position: Vec3) -> Entity {
        let entity = world.spawn();
        world.insert(entity, Transform::from_position(position));
        world.insert(entity, Hurt::new());
        entity
    }

    fn scored_grid(world: &World) -> PlacementGrid {
        let mut grid = PlacementGrid::new(Vec2::ZERO, PlacementWeights::default());
        grid.recompute(world, &flat_ground());
        grid
    }

    /// A settlement of four, all in one corner of the grid, with somewhere
    /// flat and empty to plant on.
    fn camp() -> (World, Vec<Entity>) {
        let mut world = World::new();
        let settlers = (0..4)
            .map(|i| settler_at(&mut world, Vec3::new(40.0 + i as f32 * 8.0, 0.0, 40.0)))
            .collect();
        (world, settlers)
    }

    #[test]
    fn with_nothing_lived_through_yet_the_planner_opens_nothing() {
        let (world, _) = camp();
        let grid = scored_grid(&world);
        let log = DeficitLog::new(TICKS_PER_DAY);
        let strikes = StrikeBoard::new();
        let mut mind = SettlementMind::new(TICKS_PER_DAY);

        let survey = Survey {
            tick: 0,
            world: &world,
            log: &log,
            grid: &grid,
            strikes: &strikes,
            stake: None,
        };
        assert!(mind.sweep_due(0), "tick zero is a sweep");
        assert_eq!(mind.update(&survey), None);
        assert_eq!(
            mind.ranking(&survey),
            Vec::new(),
            "four undefined readings are four non-candidates, not four zeroes"
        );
    }

    #[test]
    fn one_definable_deficit_is_opened_at_the_next_sweep_and_not_before() {
        let (world, _) = camp();
        let grid = scored_grid(&world);
        let mut log = DeficitLog::new(TICKS_PER_DAY);
        // Only one of the four is definable: everyone slept off a spot.
        log.record_night(0.0, 1.0, TICKS_PER_DAY);
        let strikes = StrikeBoard::new();
        let mut mind = SettlementMind::new(TICKS_PER_DAY);

        let survey = |tick| Survey {
            tick,
            world: &world,
            log: &log,
            grid: &grid,
            strikes: &strikes,
            stake: None,
        };

        // Warmth is definable too now (both roll over together), but at 0.0
        // it scores below shelter's 1.0 / 60. The ticks in between are not
        // sweeps and the planner does not even look.
        for tick in [TICKS_PER_DAY + 1, TICKS_PER_DAY + 899] {
            assert_eq!(mind.update(&survey(tick)), None, "tick {tick} is no sweep");
        }

        let plan = mind
            .update(&survey(900 * 2))
            .expect("a sweep with a definable deficit standing");
        assert_eq!(plan.kind(), DeficitKind::Shelter);
        assert_eq!(plan.stake().target_units(), 60);
        assert_eq!(mind.last_wake(), Some((900 * 2, Wake::Period)));
    }

    #[test]
    fn an_event_opens_the_stake_before_the_sweep_would_have() {
        let (world, _) = camp();
        let grid = scored_grid(&world);
        let mut log = DeficitLog::new(TICKS_PER_DAY);
        log.record_night(1.0, 0.0, TICKS_PER_DAY);
        let strikes = StrikeBoard::new();
        let mut mind = SettlementMind::new(TICKS_PER_DAY);

        let tick = TICKS_PER_DAY;
        assert!(!mind.sweep_due(tick), "480 is not a multiple of 900");
        let plan = mind
            .wake(
                Wake::NightLogClosed,
                &Survey {
                    tick,
                    world: &world,
                    log: &log,
                    grid: &grid,
                    strikes: &strikes,
                    stake: None,
                },
            )
            .expect("dawn is a wake of its own");
        assert_eq!(plan.kind(), DeficitKind::Warmth);
        assert_eq!(mind.last_wake(), Some((tick, Wake::NightLogClosed)));
    }

    #[test]
    fn thinking_twice_on_one_tick_yields_one_stake() {
        let (world, _) = camp();
        let grid = scored_grid(&world);
        let mut log = DeficitLog::new(TICKS_PER_DAY);
        log.record_night(1.0, 1.0, TICKS_PER_DAY);
        let strikes = StrikeBoard::new();
        let mut mind = SettlementMind::new(TICKS_PER_DAY);

        let survey = Survey {
            tick: 900,
            world: &world,
            log: &log,
            grid: &grid,
            strikes: &strikes,
            stake: None,
        };
        assert!(mind.wake(Wake::SettlerDied, &survey).is_some());
        assert_eq!(
            mind.update(&survey),
            None,
            "the sweep on the same tick has nothing left to decide"
        );
    }

    #[test]
    fn the_higher_severity_over_cost_wins_not_the_higher_severity() {
        let (world, _) = camp();
        let grid = scored_grid(&world);
        let mut log = DeficitLog::new(TICKS_PER_DAY);
        // Shelter hurts twice as much as warmth, but a hut costs six times a
        // hearth: 0.6/60 = 0.01 against 0.3/10 = 0.03.
        log.record_night(0.3, 0.6, TICKS_PER_DAY);
        let strikes = StrikeBoard::new();
        let mut mind = SettlementMind::new(TICKS_PER_DAY);

        let survey = Survey {
            tick: 900,
            world: &world,
            log: &log,
            grid: &grid,
            strikes: &strikes,
            stake: None,
        };

        let ranking = mind.ranking(&survey);
        assert_eq!(ranking.len(), 2, "only the two night readings are defined");
        assert_eq!(ranking[0].kind, DeficitKind::Warmth);
        assert_eq!(ranking[1].kind, DeficitKind::Shelter);
        assert!(
            ranking[1].severity > ranking[0].severity,
            "the loser is the one that hurts more, which is the whole point"
        );
        assert_eq!(
            mind.update(&survey).map(|p| p.kind()),
            Some(DeficitKind::Warmth)
        );
    }

    #[test]
    fn a_dearer_hearth_flips_the_same_two_readings_round() {
        let (world, _) = camp();
        let grid = scored_grid(&world);
        let mut log = DeficitLog::new(TICKS_PER_DAY);
        log.record_night(0.3, 0.6, TICKS_PER_DAY);
        let strikes = StrikeBoard::new();
        // Nothing about the settlement changed; only what a hearth costs.
        let mut mind = SettlementMind::with_tuning(
            TICKS_PER_DAY,
            PlannerTuning {
                hearth: BuildCost::new(Tag::Hard, 200),
                ..PlannerTuning::default()
            },
        );

        let plan = mind
            .update(&Survey {
                tick: 900,
                world: &world,
                log: &log,
                grid: &grid,
                strikes: &strikes,
                stake: None,
            })
            .expect("still two definable deficits");
        assert_eq!(plan.kind(), DeficitKind::Shelter);
    }

    #[test]
    fn a_stake_already_standing_stops_a_second_one_even_while_frozen() {
        let (world, _) = camp();
        let grid = scored_grid(&world);
        let mut log = DeficitLog::new(TICKS_PER_DAY);
        log.record_night(1.0, 1.0, TICKS_PER_DAY);
        let strikes = StrikeBoard::new();
        let mut mind = SettlementMind::new(TICKS_PER_DAY);

        let mut standing = Stake::new(DeficitKind::Storage, Tag::Hard, 30, TICKS_PER_DAY, 0);
        let at = |tick, stake: Option<&Stake>, mind: &mut SettlementMind| {
            mind.wake(
                Wake::StakeFinished,
                &Survey {
                    tick,
                    world: &world,
                    log: &log,
                    grid: &grid,
                    strikes: &strikes,
                    stake,
                },
            )
        };

        assert_eq!(at(900, Some(&standing), &mut mind), None, "one at a time");
        standing.freeze(900);
        assert_eq!(
            at(901, Some(&standing), &mut mind),
            None,
            "frozen is paused, not gone"
        );

        standing.thaw(902);
        let refund = standing
            .advance(902 + 2 * TICKS_PER_DAY)
            .expect("two idle sim-days");
        assert!(refund.is_empty());
        assert!(
            at(903, Some(&standing), &mut mind).is_some(),
            "abandoned frees the hands"
        );
    }

    #[test]
    fn three_strikes_takes_a_deficit_out_of_the_running_and_the_next_one_wins() {
        let (world, _) = camp();
        let grid = scored_grid(&world);
        let mut log = DeficitLog::new(TICKS_PER_DAY);
        log.record_night(1.0, 1.0, TICKS_PER_DAY);
        let mut strikes = StrikeBoard::new();
        for _ in 0..3 {
            strikes.record_abandonment(DeficitKind::Warmth);
        }
        let mut mind = SettlementMind::new(TICKS_PER_DAY);

        let survey = Survey {
            tick: 900,
            world: &world,
            log: &log,
            grid: &grid,
            strikes: &strikes,
            stake: None,
        };
        let ranking = mind.ranking(&survey);
        assert_eq!(ranking.len(), 1);
        assert_eq!(ranking[0].kind, DeficitKind::Shelter);
        assert_eq!(
            mind.update(&survey).map(|p| p.kind()),
            Some(DeficitKind::Shelter),
            "warmth outscores shelter and is still not the answer"
        );
    }

    #[test]
    fn the_planter_is_the_nearest_of_those_the_deficit_hit_hardest() {
        let mut world = World::new();
        let site = Vec2::new(100.0, 100.0);

        // Standing on the site, but this never happened to them.
        let bystander = settler_at(&mut world, Vec3::new(100.0, 0.0, 100.0));
        // Hit once, and closer than the next.
        let grazed = settler_at(&mut world, Vec3::new(110.0, 0.0, 100.0));
        world
            .get_mut::<Hurt>(grazed)
            .unwrap()
            .record(DeficitKind::Warmth);
        // Hit three times, a long way off — and still the one who goes.
        let frozen = settler_at(&mut world, Vec3::new(400.0, 0.0, 100.0));
        // Hit three times as well, further still.
        let further = settler_at(&mut world, Vec3::new(460.0, 0.0, 100.0));
        for entity in [frozen, further] {
            for _ in 0..3 {
                world
                    .get_mut::<Hurt>(entity)
                    .unwrap()
                    .record(DeficitKind::Warmth);
            }
        }

        assert_eq!(planter_for(&world, DeficitKind::Warmth, site), Some(frozen));
        assert_eq!(
            planter_for(&world, DeficitKind::Shelter, site),
            Some(bystander),
            "a different deficit is a different tally: nobody was hit, so nearest wins"
        );
        assert_ne!(planter_for(&world, DeficitKind::Warmth, site), Some(grazed));
    }

    #[test]
    fn the_planner_sends_the_hardest_hit_settler_rather_than_the_closest_one() {
        let mut world = World::new();
        // A crowd near the origin corner, so the grid's best cell lands
        // among them, and one settler well away from it.
        let near: Vec<Entity> = (0..3)
            .map(|i| settler_at(&mut world, Vec3::new(40.0 + i as f32 * 8.0, 0.0, 40.0)))
            .collect();
        let distant = settler_at(&mut world, Vec3::new(300.0, 0.0, 300.0));
        for _ in 0..5 {
            world
                .get_mut::<Hurt>(distant)
                .unwrap()
                .record(DeficitKind::Warmth);
        }

        let grid = scored_grid(&world);
        let mut log = DeficitLog::new(TICKS_PER_DAY);
        log.record_night(1.0, 0.0, TICKS_PER_DAY);
        let strikes = StrikeBoard::new();
        let mut mind = SettlementMind::new(TICKS_PER_DAY);

        let plan = mind
            .update(&Survey {
                tick: 900,
                world: &world,
                log: &log,
                grid: &grid,
                strikes: &strikes,
                stake: None,
            })
            .expect("everyone went cold");
        assert_eq!(plan.planter(), distant);
        assert!(!near.contains(&plan.planter()));
    }

    #[test]
    fn a_settlement_with_nobody_in_it_plants_nothing() {
        let world = World::new();
        let grid = scored_grid(&world);
        let mut log = DeficitLog::new(TICKS_PER_DAY);
        log.record_night(1.0, 1.0, TICKS_PER_DAY);
        let strikes = StrikeBoard::new();
        let mut mind = SettlementMind::new(TICKS_PER_DAY);

        let survey = Survey {
            tick: 900,
            world: &world,
            log: &log,
            grid: &grid,
            strikes: &strikes,
            stake: None,
        };
        assert_eq!(mind.ranking(&survey).len(), 2, "the deficits are real");
        assert_eq!(mind.update(&survey), None, "but there is nobody to send");
    }

    #[test]
    fn a_grid_with_no_ground_left_plants_nothing() {
        let (world, _) = camp();
        // Never recomputed: every cell reads as excluded.
        let grid = PlacementGrid::new(Vec2::ZERO, PlacementWeights::default());
        let mut log = DeficitLog::new(TICKS_PER_DAY);
        log.record_night(1.0, 1.0, TICKS_PER_DAY);
        let strikes = StrikeBoard::new();
        let mut mind = SettlementMind::new(TICKS_PER_DAY);

        assert_eq!(
            mind.update(&Survey {
                tick: 900,
                world: &world,
                log: &log,
                grid: &grid,
                strikes: &strikes,
                stake: None,
            }),
            None
        );
    }

    #[test]
    fn planting_spawns_a_new_entity_and_leaves_every_settler_exactly_as_it_found_them() {
        let (mut world, settlers) = camp();
        let grid = scored_grid(&world);
        let mut log = DeficitLog::new(TICKS_PER_DAY);
        log.record_night(1.0, 0.0, TICKS_PER_DAY);
        let strikes = StrikeBoard::new();
        let mut mind = SettlementMind::new(TICKS_PER_DAY);

        let plan = mind
            .update(&Survey {
                tick: 900,
                world: &world,
                log: &log,
                grid: &grid,
                strikes: &strikes,
                stake: None,
            })
            .expect("a cold night and somewhere to put a hearth");
        let site = plan.site();
        let planter = plan.planter();

        let before: Vec<(Transform, Hurt)> = settlers
            .iter()
            .map(|&e| {
                (
                    *world.get::<Transform>(e).unwrap(),
                    *world.get::<Hurt>(e).unwrap(),
                )
            })
            .collect();
        let entities_before = world.entity_count();

        let stake_entity = plan.plant(&mut world, &flat_ground());

        assert_eq!(world.entity_count(), entities_before + 1);
        assert!(!settlers.contains(&stake_entity));
        assert_eq!(
            world.get::<Stake>(stake_entity).map(|s| s.kind()),
            Some(DeficitKind::Warmth)
        );
        let placed = world.get::<Transform>(stake_entity).unwrap().position;
        assert_eq!((placed.x, placed.z), (site.x, site.y));

        for (&entity, (transform, hurt)) in settlers.iter().zip(before) {
            assert_eq!(*world.get::<Transform>(entity).unwrap(), transform);
            assert_eq!(*world.get::<Hurt>(entity).unwrap(), hurt);
        }
        assert!(
            world.get::<Stake>(planter).is_none(),
            "the settler was sent, not written to"
        );
    }

    #[test]
    fn severities_pair_each_reading_with_the_building_that_answers_it() {
        let mut log = DeficitLog::new(TICKS_PER_DAY);
        log.record_night(0.25, 0.75, TICKS_PER_DAY);
        log.record_eat_attempt(TICKS_PER_DAY, false);
        log.record_item_aged(TICKS_PER_DAY, false);

        assert_eq!(
            severities(&log),
            [
                (DeficitKind::Shelter, Some(0.75)),
                (DeficitKind::Warmth, Some(0.25)),
                (DeficitKind::Storage, Some(0.0)),
                (DeficitKind::Drying, Some(1.0)),
            ]
        );
    }

    #[test]
    fn a_severity_threshold_keeps_a_barely_felt_deficit_out_of_the_running() {
        let (world, _) = camp();
        let grid = scored_grid(&world);
        let mut log = DeficitLog::new(TICKS_PER_DAY);
        log.record_night(0.1, 0.5, TICKS_PER_DAY);
        let strikes = StrikeBoard::new();
        let mind = SettlementMind::with_tuning(
            TICKS_PER_DAY,
            PlannerTuning {
                severity_threshold: 0.25,
                ..PlannerTuning::default()
            },
        );

        let ranking = mind.ranking(&Survey {
            tick: 900,
            world: &world,
            log: &log,
            grid: &grid,
            strikes: &strikes,
            stake: None,
        });
        assert_eq!(ranking.len(), 1);
        assert_eq!(
            ranking[0].kind,
            DeficitKind::Shelter,
            "warmth would have won on score, and one settler in ten is not enough"
        );
    }

    /// `12-minds.md`'s verification check #8, as a type check rather than a
    /// habit. These coercions name the planner's whole public surface; a
    /// `&mut` to anything a settler is made of, anywhere in any of them,
    /// stops this test compiling.
    #[test]
    fn the_planner_s_signatures_cannot_reach_a_settler() {
        let _update: for<'a, 'b> fn(&'a mut SettlementMind, &'b Survey<'b>) -> Option<Plan> =
            SettlementMind::update;
        let _wake: for<'a, 'b> fn(&'a mut SettlementMind, Wake, &'b Survey<'b>) -> Option<Plan> =
            SettlementMind::wake;
        let _ranking: for<'a, 'b> fn(&'a SettlementMind, &'b Survey<'b>) -> Vec<Candidate> =
            SettlementMind::ranking;
        let _planter: fn(&World, DeficitKind, Vec2) -> Option<Entity> = planter_for;
        let _severities: fn(&DeficitLog) -> Severities = severities;
        // The one mutable thing in the module, and it is a `&mut World` that
        // spawns: see `planting_spawns_a_new_entity_and_leaves_every_settler
        // _exactly_as_it_found_them` for the run-time half of the claim.
        let _plant: fn(Plan, &mut World, &Heightfield) -> Entity = Plan::plant;
    }

    #[test]
    fn the_same_facts_decide_the_same_way_twice() {
        let (world, _) = camp();
        let grid = scored_grid(&world);
        let mut log = DeficitLog::new(TICKS_PER_DAY);
        log.record_night(0.5, 0.5, TICKS_PER_DAY);
        let strikes = StrikeBoard::new();

        let plan_of = |mind: &mut SettlementMind| {
            mind.update(&Survey {
                tick: 900,
                world: &world,
                log: &log,
                grid: &grid,
                strikes: &strikes,
                stake: None,
            })
        };
        let first = plan_of(&mut SettlementMind::new(TICKS_PER_DAY));
        let second = plan_of(&mut SettlementMind::new(TICKS_PER_DAY));
        assert_eq!(first, second, "no RNG anywhere in the settlement mind");
        assert!(first.is_some());
    }

    #[test]
    #[should_panic(expected = "at least one tick long")]
    fn a_zero_length_day_is_a_caller_bug() {
        SettlementMind::new(0);
    }
}
