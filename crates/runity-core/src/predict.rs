//! Acting now and being corrected later.
//!
//! On a shared world the server decides, and the server is 40 ms away. A
//! client that waits for permission before moving feels like it is dragging
//! the character through mud — the input lag is small in milliseconds and
//! enormous in feel. So the client moves immediately, using the same rules the
//! server will use, and keeps every input it sent. When the server's version
//! of an earlier tick arrives, the client checks it against what it predicted:
//! if they agree, the prediction was free; if they do not, it rewinds to the
//! server's state and replays everything since.
//!
//! The replay is why the simulation has to be deterministic and to run on
//! whole ticks. Replaying inputs against a different fixed step, or against
//! code that consults a shared random generator, produces a different answer
//! every time and turns every correction into a visible jolt.
//!
//! What this does *not* do is hide the correction. A snap from a rewind looks
//! bad, and the cure is presentation — ease the drawn position toward the
//! corrected one over a few frames — which belongs to the game rather than
//! here.

use std::collections::VecDeque;

/// State that can be advanced by an input, the same way on every machine.
pub trait Predictable: Clone {
    /// What drives it: a movement wish, a button, a command.
    type Input: Clone;

    /// Advance by one fixed step.
    ///
    /// Must be deterministic: the same state and input have to give the same
    /// result, or reconciliation will fight the prediction forever.
    fn step(&mut self, input: &Self::Input, dt: f32);

    /// How far apart two versions of this state are.
    ///
    /// Used with a tolerance rather than equality, because two machines
    /// computing the same thing in floating point can disagree in the last
    /// bit, and correcting for that would be a correction every tick.
    fn error(&self, other: &Self) -> f32;
}

/// One predicted tick: what was applied, and what it produced.
#[derive(Clone)]
struct Step<S: Predictable> {
    tick: u64,
    input: S::Input,
    /// The state after the input was applied.
    result: S,
}

/// What happened when the server's version arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Correction {
    /// The prediction was right; nothing moved.
    None,
    /// The prediction was wrong and the state was rewound and replayed.
    Applied,
    /// The message was older than one already applied, and was ignored.
    Stale,
}

/// Predicts locally, and reconciles with an authority that is behind.
pub struct Prediction<S: Predictable> {
    state: S,
    history: VecDeque<Step<S>>,
    /// Newest tick the authority has confirmed.
    confirmed: u64,
    /// How far apart states may be before it counts as a disagreement.
    pub tolerance: f32,
    /// How many ticks of input to keep. At sixty ticks a second, two hundred
    /// covers a round trip of three seconds, which is a bad day on a good
    /// connection.
    pub max_history: usize,
    corrections: u64,
    worst_error: f32,
}

impl<S: Predictable> core::fmt::Debug for Prediction<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Derived `Debug` would demand it of the input type, which is a
        // requirement the caller should not have to meet.
        f.debug_struct("Prediction")
            .field("confirmed", &self.confirmed)
            .field("unconfirmed", &self.history.len())
            .field("corrections", &self.corrections)
            .finish()
    }
}

impl<S: Predictable> Prediction<S> {
    /// Start predicting from a known state.
    pub fn new(state: S) -> Self {
        Self {
            state,
            history: VecDeque::new(),
            confirmed: 0,
            tolerance: 0.01,
            max_history: 200,
            corrections: 0,
            worst_error: 0.0,
        }
    }

    /// The state as the client currently believes it to be.
    pub fn state(&self) -> &S {
        &self.state
    }

    /// The state, mutably — for a teleport or a respawn, where prediction
    /// has nothing to say.
    pub fn state_mut(&mut self) -> &mut S {
        &mut self.state
    }

    /// The newest tick the authority has confirmed.
    pub fn confirmed(&self) -> u64 {
        self.confirmed
    }

    /// How many inputs are waiting to be confirmed.
    pub fn unconfirmed(&self) -> usize {
        self.history.len()
    }

    /// How many corrections have been needed, and the worst disagreement seen.
    ///
    /// Worth showing in a debug overlay: a rising count usually means the
    /// client and server are not running the same rules, which is a bug that
    /// otherwise only shows up as unexplained rubber-banding.
    pub fn corrections(&self) -> (u64, f32) {
        (self.corrections, self.worst_error)
    }

    /// Apply an input locally and remember it.
    pub fn predict(&mut self, tick: u64, input: S::Input, dt: f32) -> &S {
        self.state.step(&input, dt);
        self.history.push_back(Step {
            tick,
            input,
            result: self.state.clone(),
        });
        while self.history.len() > self.max_history {
            // Dropping the oldest unconfirmed input means a correction older
            // than the window cannot be replayed exactly — by then the
            // connection is so far gone that snapping is the honest outcome.
            self.history.pop_front();
        }
        &self.state
    }

    /// Take the authority's version of `tick` and, if it disagrees, rewind and
    /// replay everything since.
    pub fn reconcile(&mut self, tick: u64, authoritative: S, dt: f32) -> Correction {
        if tick < self.confirmed {
            return Correction::Stale;
        }
        self.confirmed = tick;

        // Inputs up to and including that tick are settled.
        let predicted = self
            .history
            .iter()
            .find(|step| step.tick == tick)
            .map(|step| step.result.clone());
        while self.history.front().is_some_and(|step| step.tick <= tick) {
            self.history.pop_front();
        }

        let error = predicted
            .as_ref()
            .map_or(f32::INFINITY, |state| state.error(&authoritative));
        self.worst_error = self
            .worst_error
            .max(if error.is_finite() { error } else { 0.0 });
        if error <= self.tolerance {
            return Correction::None;
        }

        // Rewind, then replay what the server has not seen yet.
        self.state = authoritative;
        for step in self.history.iter_mut() {
            self.state.step(&step.input, dt);
            step.result = self.state.clone();
        }
        self.corrections += 1;
        Correction::Applied
    }

    /// Throw away the prediction and start again from a known state.
    pub fn reset(&mut self, state: S, tick: u64) {
        self.state = state;
        self.history.clear();
        self.confirmed = tick;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_math::{vec3, Vec3};

    /// A character-shaped thing: position and velocity, driven by a wish.
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Mover {
        position: Vec3,
        velocity: Vec3,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Wish {
        direction: Vec3,
        jump: bool,
    }

    impl Predictable for Mover {
        type Input = Wish;

        fn step(&mut self, input: &Wish, dt: f32) {
            self.velocity = input.direction * 4.0;
            if input.jump && self.position.y <= 0.0 {
                self.velocity.y = 5.0;
            } else {
                self.velocity.y = (self.velocity.y - 9.81 * dt).max(-20.0);
            }
            self.position += self.velocity * dt;
            if self.position.y < 0.0 {
                self.position.y = 0.0;
            }
        }

        fn error(&self, other: &Self) -> f32 {
            (self.position - other.position).length()
        }
    }

    const DT: f32 = 1.0 / 60.0;

    fn walking(x: f32) -> Wish {
        Wish {
            direction: vec3(x, 0.0, 0.0),
            jump: false,
        }
    }

    fn start() -> Mover {
        Mover {
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
        }
    }

    /// What the server would compute, given the same inputs.
    fn authoritative(from: Mover, inputs: &[Wish]) -> Mover {
        let mut state = from;
        for input in inputs {
            state.step(input, DT);
        }
        state
    }

    #[test]
    fn prediction_moves_immediately_rather_than_waiting() {
        let mut prediction = Prediction::new(start());
        for tick in 1..=30 {
            prediction.predict(tick, walking(1.0), DT);
        }
        assert!(
            prediction.state().position.x > 1.9,
            "half a second of walking"
        );
        assert_eq!(
            prediction.unconfirmed(),
            30,
            "and none of it is confirmed yet"
        );
    }

    #[test]
    fn an_agreeing_server_costs_nothing() {
        let mut prediction = Prediction::new(start());
        let inputs: Vec<Wish> = (0..20).map(|_| walking(1.0)).collect();
        for (index, input) in inputs.iter().enumerate() {
            prediction.predict(index as u64 + 1, *input, DT);
        }

        // The server ran the same inputs and reached the same place.
        let server = authoritative(start(), &inputs[..10]);
        let before = *prediction.state();
        assert_eq!(prediction.reconcile(10, server, DT), Correction::None);
        assert_eq!(*prediction.state(), before, "nothing should have moved");
        assert_eq!(
            prediction.unconfirmed(),
            10,
            "and the settled inputs are forgotten"
        );
        assert_eq!(prediction.corrections().0, 0);
    }

    #[test]
    fn a_disagreeing_server_rewinds_and_replays() {
        let mut prediction = Prediction::new(start());
        let inputs: Vec<Wish> = (0..20).map(|_| walking(1.0)).collect();
        for (index, input) in inputs.iter().enumerate() {
            prediction.predict(index as u64 + 1, *input, DT);
        }

        // The server says the character was pushed back — a wall the client
        // did not know about, say.
        let mut server = authoritative(start(), &inputs[..10]);
        server.position.x -= 1.0;

        assert_eq!(prediction.reconcile(10, server, DT), Correction::Applied);

        // The replayed result is the server's state plus the ten inputs it
        // has not seen yet — not a snap to the server's position.
        let expected = authoritative(server, &inputs[10..]);
        assert!(
            prediction.state().error(&expected) < 1e-4,
            "expected {:?}, got {:?}",
            expected.position,
            prediction.state().position
        );
        assert_eq!(prediction.corrections().0, 1);
        assert!(
            prediction.corrections().1 > 0.9,
            "and the disagreement is reported"
        );
    }

    #[test]
    fn replaying_keeps_the_inputs_the_server_has_not_seen() {
        // The point of replay: recent input must not be lost. A client that
        // snapped to the server's position would throw away every keypress
        // made during the round trip.
        let mut prediction = Prediction::new(start());
        for tick in 1..=10 {
            prediction.predict(tick, walking(1.0), DT);
        }
        for tick in 11..=20 {
            prediction.predict(tick, walking(-1.0), DT);
        }

        let mut server = authoritative(start(), &[walking(1.0); 10]);
        server.position.z += 2.0; // a disagreement the client cannot explain
        prediction.reconcile(10, server, DT);

        // The ten backwards steps still happened.
        assert!(prediction.state().position.x < server.position.x);
        assert!(
            (prediction.state().position.z - 2.0).abs() < 1e-4,
            "and the server's z survived"
        );
    }

    #[test]
    fn a_small_disagreement_is_tolerated_rather_than_corrected() {
        // Two machines doing the same arithmetic can differ in the last bit,
        // and correcting for that would mean a correction every tick.
        let mut prediction = Prediction::new(start());
        prediction.tolerance = 0.05;
        let inputs: Vec<Wish> = (0..10).map(|_| walking(1.0)).collect();
        for (index, input) in inputs.iter().enumerate() {
            prediction.predict(index as u64 + 1, *input, DT);
        }

        let mut server = authoritative(start(), &inputs[..5]);
        server.position.x += 0.001;
        assert_eq!(prediction.reconcile(5, server, DT), Correction::None);
        assert_eq!(prediction.corrections().0, 0);
    }

    #[test]
    fn an_old_message_is_ignored() {
        let mut prediction = Prediction::new(start());
        for tick in 1..=20 {
            prediction.predict(tick, walking(1.0), DT);
        }
        prediction.reconcile(15, authoritative(start(), &vec![walking(1.0); 15]), DT);
        assert_eq!(prediction.confirmed(), 15);

        // A snapshot for tick 10 turning up late must not rewind the world to
        // it; it describes a past that has already been settled.
        assert_eq!(prediction.reconcile(10, start(), DT), Correction::Stale);
        assert_eq!(prediction.confirmed(), 15);
        assert!(prediction.state().position.x > 0.5);
    }

    #[test]
    fn a_state_for_an_unpredicted_tick_is_taken_as_truth() {
        // A client that has just joined has predicted nothing, so there is
        // nothing to compare against and the server is simply right.
        let mut prediction = Prediction::new(start());
        let server = Mover {
            position: vec3(30.0, 0.0, 5.0),
            velocity: Vec3::ZERO,
        };
        assert_eq!(prediction.reconcile(500, server, DT), Correction::Applied);
        assert_eq!(prediction.state().position, vec3(30.0, 0.0, 5.0));
    }

    #[test]
    fn history_is_bounded() {
        let mut prediction = Prediction::new(start());
        prediction.max_history = 32;
        for tick in 1..=1_000 {
            prediction.predict(tick, walking(1.0), DT);
        }
        assert_eq!(
            prediction.unconfirmed(),
            32,
            "an unanswering server must not grow the client"
        );
    }

    #[test]
    fn a_reset_starts_again_cleanly() {
        let mut prediction = Prediction::new(start());
        for tick in 1..=10 {
            prediction.predict(tick, walking(1.0), DT);
        }
        let respawn = Mover {
            position: vec3(-5.0, 0.0, 0.0),
            velocity: Vec3::ZERO,
        };
        prediction.reset(respawn, 100);

        assert_eq!(*prediction.state(), respawn);
        assert_eq!(prediction.unconfirmed(), 0);
        assert_eq!(prediction.confirmed(), 100);
    }

    #[test]
    fn a_client_that_agrees_with_the_server_never_drifts() {
        // The end-to-end property: a client predicting the same rules as the
        // server, told the truth one round trip late, stays exactly with it.
        let mut prediction = Prediction::new(start());
        let mut server = start();
        let mut sent: Vec<(u64, Wish)> = Vec::new();
        let latency = 6; // ticks each way

        for tick in 1..=200u64 {
            let input = if tick % 40 < 20 {
                walking(1.0)
            } else {
                walking(-0.5)
            };
            prediction.predict(tick, input, DT);
            sent.push((tick, input));

            // The server applies what arrived, and its state comes back later.
            if tick > latency as u64 {
                let (server_tick, input) = sent[(tick - latency as u64 - 1) as usize];
                server.step(&input, DT);
                let correction = prediction.reconcile(server_tick, server, DT);
                assert_eq!(correction, Correction::None, "at tick {tick}");
            }
        }
        assert_eq!(prediction.corrections().0, 0);
        assert!(prediction.unconfirmed() <= latency + 1);
    }
}
