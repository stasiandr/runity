//! Twelve settlers, two stakes and a week, decided one tick at a time.
//!
//! The unit tests next to [`runity_core::mind`] pin each scoring term down on
//! its own. This one asks the only question those cannot: left alone for a
//! sim-week, does a settlement of these minds get on with things, or does it
//! dither?
//!
//! The answer is deliberately not an assertion on a number. A switch count is
//! exactly the sort of figure that moves the moment anybody touches a
//! constant, and a test that pins it would be a test of the tuning rather
//! than of the mind. What is asserted is the shape: that settlers switch at
//! all, that they do not switch constantly, that the run reproduces itself
//! bit for bit, and that hysteresis is what makes the difference. The numbers
//! themselves are printed, for a human deciding whether the tuning is any
//! good.

use runity_core::economy::Tag;
use runity_core::mind::{
    AtHand, Chore, MindTuning, OneStep, Settlement, SettlerSpec, SwitchReport,
};
use runity_core::needs::{Needs, NeedsRates};
use runity_math::Vec3;

/// Twenty-four ticks to the sim-hour at a 20 Hz step: a day is 480 ticks and
/// 24 seconds of simulated time.
const TICKS_PER_DAY: u64 = 480;
const SECONDS_PER_TICK: f32 = 1.0 / 20.0;
const DAYS: u64 = 7;
const SETTLERS: usize = 12;

/// Needs scaled to this run's short sim-day: hungry in about a day, cold in
/// most of one, tired in two. The default [`NeedsRates`] are written for a
/// day measured in real minutes and would not move at all in 24 seconds.
fn appetite() -> Needs {
    Needs::new(NeedsRates {
        hunger_decay: 1.0 / 26.0,
        warmth_decay: 1.0 / 18.0,
        rest_decay: 1.0 / 44.0,
        warm_gain: 1.0 / 3.0,
        sleep_gain: 1.0 / 5.0,
    })
}

/// The camp of `12-minds.md` part 0: twelve settlers around one fire, with
/// the hearth's firewood to keep up and a stake asking for something sharp
/// that only some of them can make.
fn camp(tuning: MindTuning) -> Settlement {
    let mut settlement = Settlement::new(tuning, TICKS_PER_DAY, SECONDS_PER_TICK);
    settlement.add_chore(Chore::new(Vec3::new(0.0, 0.0, 0.0), 0.75));
    settlement.add_chore(Chore::new(Vec3::new(-22.0, 0.0, 34.0), 0.9).wanting(Tag::Sharp));

    for i in 0..SETTLERS {
        let angle = i as f32;
        let spot = Vec3::new(angle * 2.5 - 14.0, 0.0, (i % 5) as f32 * 4.0);
        // A third of the camp is carrying something hard and knows how to
        // knap it; the rest have no way into the second stake at all.
        let at_hand = if i % 3 == 0 {
            AtHand::new([Tag::Hard], [OneStep::new(Tag::Hard, Tag::Sharp)])
        } else {
            AtHand::holding([Tag::Warm])
        };
        settlement.add_settler(
            SettlerSpec::new(spot)
                .able(i as f32 / SETTLERS as f32, 0.55 + (i % 4) as f32 * 0.15)
                .feeling(appetite())
                .carrying(at_hand),
        );
    }
    settlement
}

fn report_line(name: &str, report: &SwitchReport) -> String {
    format!("{name:>10}: {report}")
}

#[test]
fn a_week_of_twelve_minds_switches_often_enough_to_live_and_rarely_enough_to_work() {
    let report = camp(MindTuning::default()).run_days(DAYS);
    println!("{}", report_line("default", &report));

    assert_eq!(
        report.count(),
        SETTLERS * DAYS as usize,
        "one sample per settler per day"
    );
    assert!(
        report.median() > 0,
        "a settler who never changes their mind in a whole day is not deciding anything: {report}"
    );
    assert!(
        report.p90() <= TICKS_PER_DAY as u32 / 8,
        "the camp is dithering: {report}"
    );
    assert!(report.median() <= report.p90() && report.p90() <= report.max());
}

#[test]
fn the_same_week_run_twice_is_the_same_week() {
    let first = camp(MindTuning::default()).run_days(DAYS);
    let second = camp(MindTuning::default()).run_days(DAYS);
    assert_eq!(
        first, second,
        "the staggered recompute is a counter, not a die roll"
    );
}

#[test]
fn hysteresis_is_what_keeps_the_switch_count_down() {
    // Both camps reconsider on every single tick, so the staggered sweep
    // cannot be what separates them — only §3.3 can.
    let every_tick = MindTuning {
        recompute_period: 1,
        ..MindTuning::default()
    };
    let sticky = camp(every_tick).run_days(DAYS);
    let loose = camp(MindTuning {
        min_activity_ticks: 1,
        stickiness_bonus: 0.0,
        ..every_tick
    })
    .run_days(DAYS);

    println!("{}", report_line("sticky", &sticky));
    println!("{}", report_line("loose", &loose));

    assert!(
        loose.median() > sticky.median() && loose.max() > sticky.max(),
        "without §3.3 the same camp should visibly churn: {sticky} vs {loose}"
    );
}

#[test]
fn a_longer_minimum_hold_costs_switches_monotonically() {
    let mut previous = u32::MAX;
    for hold in [1_u64, 10, 40, 120] {
        // Every tick again: with the default sixty-tick sweep a settler gets
        // eight chances a day to change their mind, and a hold shorter than
        // the sweep has nothing left to stop.
        let report = camp(MindTuning {
            min_activity_ticks: hold,
            recompute_period: 1,
            ..MindTuning::default()
        })
        .run_days(DAYS);
        println!("{}", report_line(&format!("hold {hold}"), &report));
        assert!(
            report.max() <= previous,
            "holding longer should never switch more: {report}"
        );
        previous = report.max();
    }
}

#[test]
fn the_staggered_sweep_alone_already_holds_the_count_down() {
    let swept = camp(MindTuning::default()).run_days(DAYS);
    let every_tick = camp(MindTuning {
        recompute_period: 1,
        ..MindTuning::default()
    })
    .run_days(DAYS);

    println!("{}", report_line("swept", &swept));
    println!("{}", report_line("every tick", &every_tick));

    assert!(
        swept.max() <= every_tick.max(),
        "thinking less often cannot mean switching more: {swept} vs {every_tick}"
    );
}
