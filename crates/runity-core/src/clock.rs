//! World time: ticks, speed, pause, calendar and catch-up.
//!
//! [`Time`](crate::Time) measures frames — how long the last one took, how
//! smooth we are. That is the wrong clock for a world that keeps developing
//! while nobody watches. A settlement growing into a city cares about *world*
//! time, which has to be:
//!
//! * **Integral.** The simulation advances in whole ticks, never in "0.0167
//!   seconds". Two machines that have run the same number of ticks are in the
//!   same state; floating-point frame deltas could never promise that.
//! * **Detached from the frame rate.** Ten ticks a second, rendered at
//!   whatever the display manages. A slow machine renders less, not *less
//!   world*.
//! * **Scalable and pausable.** A strategy layer wants ×1, ×4, and a pause
//!   that genuinely stops history rather than merely hiding it.
//! * **Skippable in bulk.** When a save is loaded after a week away, or a
//!   distant region has not been simulated in detail for an hour, the missing
//!   ticks must be accountable so the world can be brought forward coarsely
//!   rather than pretending nothing happened.

use runity_serialize::{
    serializable, serializable_enum, Deserialize, Reader, Result, Serialize, Writer,
};

/// The simulation clock: converts real seconds into whole world ticks.
///
/// Hold one per world. The tick counter — not any float — is the world's
/// position in time, and it is what a save stores and what two machines
/// compare.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldClock {
    tick: u64,
    /// Scaled real seconds that have not yet become a tick.
    accumulator: f64,
    /// Real seconds per tick at speed 1.0. Part of the world's rules: change
    /// it and the same sequence of real time produces a different history.
    seconds_per_tick: f64,
    speed: f32,
    paused: bool,
    /// Ceiling on how many ticks a single [`WorldClock::advance`] may hand
    /// out, so one long stall cannot freeze the frame trying to catch up.
    pub max_ticks_per_update: u32,
    /// Ticks handed out since the last `advance`.
    issued: u32,
    /// The world's calendar, for turning ticks into dates.
    pub calendar: Calendar,
}

impl Default for WorldClock {
    fn default() -> Self {
        Self::new(Self::DEFAULT_TICK_RATE)
    }
}

impl WorldClock {
    /// Ten ticks a second: fine enough that an order feels immediate, coarse
    /// enough that a few thousand agents are affordable on one thread.
    pub const DEFAULT_TICK_RATE: f64 = 10.0;

    /// The speeds a strategy layer usually offers.
    pub const SPEEDS: [f32; 4] = [1.0, 2.0, 4.0, 8.0];

    /// A clock running at `ticks_per_second` of real time at speed 1.0.
    pub fn new(ticks_per_second: f64) -> Self {
        Self {
            tick: 0,
            accumulator: 0.0,
            seconds_per_tick: if ticks_per_second > 0.0 {
                1.0 / ticks_per_second
            } else {
                0.1
            },
            speed: 1.0,
            paused: false,
            max_ticks_per_update: 64,
            issued: 0,
            calendar: Calendar::DEFAULT,
        }
    }

    /// The same clock starting from a given tick — for loading a save.
    pub fn at_tick(mut self, tick: u64) -> Self {
        self.tick = tick;
        self
    }

    /// The same clock with a different calendar.
    pub fn with_calendar(mut self, calendar: Calendar) -> Self {
        self.calendar = calendar;
        self
    }

    // ------------------------------------------------------------- position

    /// Ticks elapsed since the world began. This *is* the world's clock.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// The current date according to the world's calendar.
    pub fn date(&self) -> Date {
        self.calendar.date(self.tick)
    }

    /// How far the frame is between the last tick and the next, in `[0, 1)`.
    ///
    /// Render with this: at ten ticks a second, drawing agents at their last
    /// tick position makes them visibly stutter, while interpolating between
    /// the previous and current position looks continuous.
    pub fn interpolation(&self) -> f32 {
        if self.seconds_per_tick <= 0.0 {
            return 0.0;
        }
        ((self.accumulator / self.seconds_per_tick) as f32).clamp(0.0, 1.0)
    }

    /// Real seconds one tick takes at speed 1.0.
    pub fn seconds_per_tick(&self) -> f64 {
        self.seconds_per_tick
    }

    /// Ticks per second of real time at speed 1.0.
    pub fn tick_rate(&self) -> f64 {
        1.0 / self.seconds_per_tick
    }

    // ---------------------------------------------------------------- speed

    /// The current time scale.
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Set the time scale. Negative values are clamped to zero — the world
    /// does not run backwards.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.max(0.0);
    }

    /// Move to the next entry of [`WorldClock::SPEEDS`], wrapping around.
    pub fn cycle_speed(&mut self) {
        let index = Self::SPEEDS
            .iter()
            .position(|s| *s >= self.speed)
            .unwrap_or(0);
        let next = (index + 1) % Self::SPEEDS.len();
        self.speed = Self::SPEEDS[next];
    }

    /// Stop history. Any partial tick is kept, so unpausing resumes rather
    /// than restarting.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Let history run again.
    pub fn resume(&mut self) {
        self.paused = false;
    }

    /// Flip between paused and running.
    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }

    /// Whether the world is paused (explicitly, or by a zero time scale).
    pub fn is_paused(&self) -> bool {
        self.paused || self.speed == 0.0
    }

    // -------------------------------------------------------------- driving

    /// Feed the clock one frame of real time.
    ///
    /// Then drain it with [`WorldClock::next_tick`]:
    ///
    /// ```
    /// # use runity_core::WorldClock;
    /// # let mut clock = WorldClock::new(10.0);
    /// # let mut simulated = 0;
    /// clock.advance(1.0 / 60.0);
    /// while let Some(_tick) = clock.next_tick() {
    ///     simulated += 1; // world.tick()
    /// }
    /// ```
    pub fn advance(&mut self, real_delta: f32) {
        self.issued = 0;
        if self.is_paused() {
            return;
        }
        // A negative or NaN delta would run the world backwards or poison the
        // accumulator; neither is a state worth having.
        let delta = if real_delta.is_finite() {
            real_delta.max(0.0)
        } else {
            0.0
        };
        self.accumulator += f64::from(delta) * f64::from(self.speed);
    }

    /// Take the next whole tick, if one is due and the per-frame budget is
    /// not spent. Returns the index of the tick to simulate.
    pub fn next_tick(&mut self) -> Option<u64> {
        if self.is_paused() || self.issued >= self.max_ticks_per_update {
            return None;
        }
        if self.accumulator < self.seconds_per_tick {
            return None;
        }
        self.accumulator -= self.seconds_per_tick;
        self.issued += 1;
        let index = self.tick;
        self.tick += 1;
        Some(index)
    }

    /// Ticks still owed, beyond the ones already handed out this frame.
    ///
    /// Growing steadily means the simulation cannot keep up at this speed —
    /// the moment to drop distant regions to a coarser level of detail, or to
    /// tell the player that ×8 is more than this world can do.
    pub fn backlog(&self) -> u64 {
        if self.seconds_per_tick <= 0.0 {
            return 0;
        }
        (self.accumulator / self.seconds_per_tick) as u64
    }

    /// Throw away the backlog, keeping the partial tick.
    ///
    /// Time is lost on purpose: better a world that skips a second than one
    /// that spends the next minute catching up on a second.
    pub fn drop_backlog(&mut self) {
        self.accumulator %= self.seconds_per_tick;
    }

    /// Advance the counter by `ticks` without handing them out.
    ///
    /// For progression that is applied in bulk rather than simulated step by
    /// step: a region nobody has visited for an hour, or a save resumed after
    /// a week. The caller is expected to apply the aggregate effect of those
    /// ticks itself.
    pub fn skip(&mut self, ticks: u64) -> u64 {
        self.tick = self.tick.saturating_add(ticks);
        ticks
    }

    /// How many ticks `real_seconds` of absence are worth at the current
    /// speed — the offline-progression question.
    pub fn ticks_for(&self, real_seconds: f64) -> u64 {
        if self.seconds_per_tick <= 0.0 || real_seconds <= 0.0 {
            return 0;
        }
        (real_seconds * f64::from(self.speed) / self.seconds_per_tick) as u64
    }

    /// Skip forward by however many ticks `real_seconds` away are worth, and
    /// report the number so the world can be brought forward in bulk.
    pub fn fast_forward(&mut self, real_seconds: f64) -> u64 {
        self.skip(self.ticks_for(real_seconds))
    }
}

/// How ticks map onto days, years and seasons.
///
/// Every field is part of the world's rules rather than a display setting: a
/// save records them, because a world with 90-day seasons is not the same
/// world with a different UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Calendar {
    /// Ticks in one world day.
    pub ticks_per_day: u64,
    /// Days in one world year.
    pub days_per_year: u64,
}

impl Calendar {
    /// One tick per world minute, a 360-day year of four 90-day seasons. At
    /// the default ten ticks a second that makes a day about two and a half
    /// real minutes.
    pub const DEFAULT: Calendar = Calendar {
        ticks_per_day: 24 * 60,
        days_per_year: 360,
    };

    /// A calendar with the given shape, guarding against zero-length days.
    pub fn new(ticks_per_day: u64, days_per_year: u64) -> Self {
        Self {
            ticks_per_day: ticks_per_day.max(1),
            days_per_year: days_per_year.max(1),
        }
    }

    /// Break a tick down into year, day and time of day.
    pub fn date(&self, tick: u64) -> Date {
        let ticks_per_day = self.ticks_per_day.max(1);
        let days_per_year = self.days_per_year.max(1);
        let total_days = tick / ticks_per_day;
        let day = total_days % days_per_year;
        Date {
            year: total_days / days_per_year,
            day,
            tick_of_day: tick % ticks_per_day,
            ticks_per_day,
            season: Season::of(day, days_per_year),
        }
    }

    /// The first tick of a given year and day — the inverse of
    /// [`Calendar::date`].
    pub fn tick_of(&self, year: u64, day: u64, tick_of_day: u64) -> u64 {
        (year * self.days_per_year.max(1) + day) * self.ticks_per_day.max(1) + tick_of_day
    }

    /// Ticks in one world year.
    pub fn ticks_per_year(&self) -> u64 {
        self.ticks_per_day.max(1) * self.days_per_year.max(1)
    }
}

impl Default for Calendar {
    fn default() -> Self {
        Calendar::DEFAULT
    }
}

/// A point in world time, as people in the world would describe it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
    /// Years since the world began.
    pub year: u64,
    /// Day within the year, counting from zero.
    pub day: u64,
    /// Ticks since midnight.
    pub tick_of_day: u64,
    /// Ticks in a day, carried so the time-of-day helpers work standalone.
    pub ticks_per_day: u64,
    /// Which season `day` falls in.
    pub season: Season,
}

impl Date {
    /// Position within the day, `0.0` at midnight and `0.5` at noon.
    ///
    /// This is what drives the sun: hand it to the sky's elevation and a
    /// world day becomes a lighting cycle for free.
    pub fn day_fraction(&self) -> f32 {
        if self.ticks_per_day == 0 {
            return 0.0;
        }
        self.tick_of_day as f32 / self.ticks_per_day as f32
    }

    /// Hour of the day, 0–23.
    pub fn hour(&self) -> u32 {
        (self.day_fraction() * 24.0) as u32 % 24
    }

    /// Minute within the hour, 0–59.
    pub fn minute(&self) -> u32 {
        let minutes = self.day_fraction() * 24.0 * 60.0;
        (minutes as u32) % 60
    }

    /// Whether it is dark out: before 06:00 or after 20:00.
    pub fn is_night(&self) -> bool {
        let f = self.day_fraction();
        !(0.25..0.833_333_3).contains(&f)
    }
}

/// The four seasons, derived from the day of the year.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Season {
    /// Planting.
    Spring,
    /// Growth.
    Summer,
    /// Harvest.
    Autumn,
    /// Scarcity — the season a settlement has to have prepared for.
    Winter,
}

impl Season {
    /// Which quarter of the year `day` falls in.
    pub fn of(day: u64, days_per_year: u64) -> Season {
        let quarter = day.saturating_mul(4) / days_per_year.max(1);
        match quarter {
            0 => Season::Spring,
            1 => Season::Summer,
            2 => Season::Autumn,
            _ => Season::Winter,
        }
    }

    /// The season's name, for the UI.
    pub fn name(&self) -> &'static str {
        match self {
            Season::Spring => "spring",
            Season::Summer => "summer",
            Season::Autumn => "autumn",
            Season::Winter => "winter",
        }
    }
}

serializable!(Calendar {
    ticks_per_day,
    days_per_year
});
serializable_enum!(Season {
    0 => Spring,
    1 => Summer,
    2 => Autumn,
    3 => Winter,
});

/// The clock is written by hand rather than through the macro because the
/// per-frame budget is a local setting, not part of the world: a save must
/// not carry one machine's frame pacing onto another.
impl Serialize for WorldClock {
    fn serialize(&self, writer: &mut Writer) {
        writer
            .write(&self.tick)
            .f64(self.accumulator)
            .f64(self.seconds_per_tick)
            .f32(self.speed)
            .bool(self.paused)
            .write(&self.calendar);
    }
}

impl Deserialize for WorldClock {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        Ok(WorldClock {
            tick: reader.read()?,
            accumulator: reader.f64()?,
            seconds_per_tick: reader.f64()?,
            speed: reader.f32()?,
            paused: reader.bool()?,
            max_ticks_per_update: 64,
            issued: 0,
            calendar: reader.read()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_serialize::{from_bytes, to_bytes};

    /// Run `frames` frames of `delta` seconds each and count the ticks.
    fn simulate(clock: &mut WorldClock, frames: usize, delta: f32) -> u64 {
        let mut ticks = 0;
        for _ in 0..frames {
            clock.advance(delta);
            while clock.next_tick().is_some() {
                ticks += 1;
            }
        }
        ticks
    }

    #[test]
    fn ten_ticks_a_second_means_ten_ticks_a_second() {
        let mut clock = WorldClock::new(10.0);
        assert_eq!(simulate(&mut clock, 60, 1.0 / 60.0), 10);
        assert_eq!(clock.tick(), 10);
    }

    #[test]
    fn the_frame_rate_does_not_change_the_world() {
        // The whole point of a separate clock: a machine rendering at 30 fps
        // and one at 144 fps must simulate the same number of ticks per
        // second of real time.
        for fps in [24.0f32, 30.0, 60.0, 144.0, 240.0] {
            let mut clock = WorldClock::new(10.0);
            let frames = (fps * 10.0) as usize;
            let ticks = simulate(&mut clock, frames, 1.0 / fps);
            assert_eq!(ticks, 100, "at {fps} fps");
        }
    }

    #[test]
    fn ticks_do_not_drift_over_a_long_session() {
        // Ten real minutes at an awkward frame time. A f32 accumulator would
        // have visibly lost time by here.
        let mut clock = WorldClock::new(10.0);
        let ticks = simulate(&mut clock, 36_000, 1.0 / 60.0);
        assert_eq!(ticks, 6_000);
    }

    #[test]
    fn the_same_deltas_always_give_the_same_ticks() {
        let deltas: Vec<f32> = (0..500).map(|i| 0.008 + (i % 7) as f32 * 0.003).collect();
        let run = || {
            let mut clock = WorldClock::new(10.0);
            for delta in &deltas {
                clock.advance(*delta);
                while clock.next_tick().is_some() {}
            }
            (clock.tick(), clock.interpolation())
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn speed_scales_history_and_zero_stops_it() {
        let mut clock = WorldClock::new(10.0);
        clock.set_speed(4.0);
        assert_eq!(simulate(&mut clock, 60, 1.0 / 60.0), 40);

        clock.set_speed(0.0);
        assert!(clock.is_paused());
        assert_eq!(simulate(&mut clock, 60, 1.0 / 60.0), 0);

        // Running backwards is not a thing.
        clock.set_speed(-3.0);
        assert_eq!(clock.speed(), 0.0);
    }

    #[test]
    fn pausing_keeps_the_partial_tick() {
        let mut clock = WorldClock::new(10.0);
        clock.advance(0.05); // half a tick
        assert!(clock.next_tick().is_none());

        clock.pause();
        assert_eq!(
            simulate(&mut clock, 100, 1.0 / 60.0),
            0,
            "a pause stops history"
        );
        assert_eq!(clock.tick(), 0);

        clock.resume();
        clock.advance(0.05);
        assert_eq!(
            clock.next_tick(),
            Some(0),
            "the half tick from before still counts"
        );
    }

    #[test]
    fn speed_cycles_through_the_presets() {
        let mut clock = WorldClock::new(10.0);
        assert_eq!(clock.speed(), 1.0);
        clock.cycle_speed();
        assert_eq!(clock.speed(), 2.0);
        clock.cycle_speed();
        assert_eq!(clock.speed(), 4.0);
        clock.cycle_speed();
        assert_eq!(clock.speed(), 8.0);
        clock.cycle_speed();
        assert_eq!(clock.speed(), 1.0);
    }

    #[test]
    fn a_stall_is_capped_but_no_time_is_lost() {
        let mut clock = WorldClock::new(10.0);
        clock.max_ticks_per_update = 8;

        // Five seconds of stall: fifty ticks are owed.
        clock.advance(5.0);
        let mut first_frame = 0;
        while clock.next_tick().is_some() {
            first_frame += 1;
        }
        assert_eq!(first_frame, 8, "the budget bounds one frame's catch-up");
        assert_eq!(clock.backlog(), 42, "the rest is still owed, not discarded");

        // Subsequent frames work it off eight at a time.
        let recovered = simulate(&mut clock, 6, 0.0);
        assert_eq!(recovered, 42);
        assert_eq!(clock.tick(), 50);
        assert_eq!(clock.backlog(), 0);
    }

    #[test]
    fn a_backlog_can_be_dropped_on_purpose() {
        let mut clock = WorldClock::new(10.0);
        clock.advance(3.0);
        assert_eq!(clock.backlog(), 30);
        clock.drop_backlog();
        assert_eq!(clock.backlog(), 0);
        assert!(clock.next_tick().is_none());
        assert_eq!(
            clock.tick(),
            0,
            "dropping the backlog does not advance the world"
        );
    }

    #[test]
    fn interpolation_crosses_the_gap_between_ticks() {
        let mut clock = WorldClock::new(10.0);
        assert_eq!(clock.interpolation(), 0.0);

        clock.advance(0.025);
        assert!((clock.interpolation() - 0.25).abs() < 1e-5);

        clock.advance(0.05);
        assert!((clock.interpolation() - 0.75).abs() < 1e-5);

        // Crossing a tick boundary wraps back toward zero rather than
        // running past one, which would make rendering overshoot.
        clock.advance(0.05);
        assert_eq!(clock.next_tick(), Some(0));
        assert!(clock.interpolation() < 1.0);
        assert!((clock.interpolation() - 0.25).abs() < 1e-5);
    }

    #[test]
    fn absurd_deltas_do_not_poison_the_clock() {
        let mut clock = WorldClock::new(10.0);
        clock.advance(-5.0);
        assert_eq!(clock.backlog(), 0);
        clock.advance(f32::NAN);
        assert_eq!(clock.backlog(), 0);
        clock.advance(f32::INFINITY);
        assert_eq!(clock.backlog(), 0);
        clock.advance(0.1);
        assert_eq!(
            clock.next_tick(),
            Some(0),
            "and the clock still works afterwards"
        );
    }

    #[test]
    fn offline_time_is_convertible_into_ticks() {
        // A save resumed a week later: the world owes seven days of history.
        let mut clock = WorldClock::new(10.0);
        let week = 7.0 * 24.0 * 3600.0;
        assert_eq!(clock.ticks_for(week), 6_048_000);

        let skipped = clock.fast_forward(week);
        assert_eq!(skipped, 6_048_000);
        assert_eq!(clock.tick(), 6_048_000);
        // Skipping does not hand those ticks to the simulation — the caller
        // applies them in bulk.
        assert!(clock.next_tick().is_none());
    }

    #[test]
    fn the_calendar_turns_ticks_into_dates() {
        let calendar = Calendar::DEFAULT;
        let date = calendar.date(0);
        assert_eq!(
            (date.year, date.day, date.hour(), date.minute()),
            (0, 0, 0, 0)
        );
        assert_eq!(date.season, Season::Spring);

        // 1440 ticks a day, one per minute.
        let noon = calendar.date(12 * 60);
        assert_eq!(noon.hour(), 12);
        assert!((noon.day_fraction() - 0.5).abs() < 1e-6);

        let evening = calendar.date(20 * 60 + 30);
        assert_eq!((evening.hour(), evening.minute()), (20, 30));

        let next_day = calendar.date(calendar.ticks_per_day);
        assert_eq!((next_day.year, next_day.day, next_day.hour()), (0, 1, 0));

        let next_year = calendar.date(calendar.ticks_per_year());
        assert_eq!((next_year.year, next_year.day), (1, 0));
    }

    #[test]
    fn dates_and_ticks_convert_both_ways() {
        let calendar = Calendar::new(1440, 360);
        for tick in [0u64, 1, 1439, 1440, 100_000, 5_184_000, 9_999_999] {
            let date = calendar.date(tick);
            assert_eq!(
                calendar.tick_of(date.year, date.day, date.tick_of_day),
                tick
            );
        }
    }

    #[test]
    fn seasons_split_the_year_into_quarters() {
        let calendar = Calendar::new(100, 360);
        assert_eq!(calendar.date(0).season, Season::Spring);
        assert_eq!(calendar.date(89 * 100).season, Season::Spring);
        assert_eq!(calendar.date(90 * 100).season, Season::Summer);
        assert_eq!(calendar.date(180 * 100).season, Season::Autumn);
        assert_eq!(calendar.date(270 * 100).season, Season::Winter);
        assert_eq!(calendar.date(359 * 100).season, Season::Winter);
        // The next year starts over.
        assert_eq!(calendar.date(360 * 100).season, Season::Spring);
        assert_eq!(Season::Winter.name(), "winter");
    }

    #[test]
    fn night_covers_the_dark_hours() {
        let calendar = Calendar::DEFAULT;
        assert!(calendar.date(2 * 60).is_night());
        assert!(!calendar.date(9 * 60).is_night());
        assert!(!calendar.date(19 * 60).is_night());
        assert!(calendar.date(22 * 60).is_night());
    }

    #[test]
    fn a_degenerate_calendar_still_answers() {
        // Nothing here should divide by zero, whatever a save contains.
        let calendar = Calendar {
            ticks_per_day: 0,
            days_per_year: 0,
        };
        let date = calendar.date(123);
        assert_eq!(date.year, 123);
        assert_eq!(calendar.ticks_per_year(), 1);
    }

    #[test]
    fn a_saved_clock_resumes_exactly_where_it_stopped() {
        let mut clock = WorldClock::new(10.0);
        clock.set_speed(2.0);
        clock.calendar = Calendar::new(600, 100);
        simulate(&mut clock, 137, 1.0 / 60.0);
        clock.advance(0.017); // leave a partial tick behind

        let bytes = to_bytes(&clock);
        let restored: WorldClock = from_bytes(&bytes).unwrap();
        assert_eq!(restored, clock);
        assert_eq!(restored.date(), clock.date());

        // And it keeps producing the same history as the original would.
        let mut original = clock.clone();
        let mut loaded = restored;
        assert_eq!(
            simulate(&mut original, 60, 1.0 / 60.0),
            simulate(&mut loaded, 60, 1.0 / 60.0)
        );
        assert_eq!(original.tick(), loaded.tick());
    }

    #[test]
    fn a_paused_clock_stays_paused_across_a_save() {
        let mut clock = WorldClock::new(10.0);
        clock.pause();
        let restored: WorldClock = from_bytes(&to_bytes(&clock)).unwrap();
        assert!(restored.is_paused());
    }
}
