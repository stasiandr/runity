//! What a settler can see of the common work around them, and what they are
//! carrying to do it with.

use crate::economy::Tag;
use crate::world::Entity;

/// One piece of common work as a single settler sees it — a stake
/// ([`crate::economy::Stake`]) or a standing chore like firewood to the
/// hearth. Nothing here is a property of the work alone: severity and the
/// material asked for are, but skill, disposition and distance are this
/// settler's, which is why two settlers looking at the same stake score it
/// differently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkCandidate {
    /// What the work is: the stake, or the entity the chore belongs to. This
    /// is also the handle stickiness watches — work whose target has left the
    /// list has disappeared, and that pierces stickiness.
    pub target: Entity,
    /// How badly it is wanted, `[0, 1]`. For a stake this is the deficit's
    /// severity out of [`crate::needs::DeficitLog`].
    pub severity: f32,
    /// How good this settler already is at it, `[0, 1]` — the self-reinforcing
    /// loop of `02-settlers.md`, "Навык как следствие".
    pub skill: f32,
    /// How drawn this settler is to it, `[0, 1]`.
    pub disposition: f32,
    /// Metres between the settler and the work.
    pub distance: f32,
    /// The property the work asks for (`05-economy.md` §6.1 — a property and
    /// an amount, never a named material), or `None` for work that asks for
    /// nothing but hands.
    pub wants: Option<Tag>,
}

impl WorkCandidate {
    /// A piece of work that asks for nothing but hands, at `distance` metres.
    pub fn new(target: Entity, severity: f32, distance: f32) -> Self {
        Self {
            target,
            severity,
            skill: 0.0,
            disposition: 1.0,
            distance,
            wants: None,
        }
    }

    /// The same work, asking for `tag`.
    pub fn wanting(mut self, tag: Tag) -> Self {
        self.wants = Some(tag);
        self
    }

    /// The same work, as seen by a settler of this skill and disposition.
    pub fn by(mut self, skill: f32, disposition: f32) -> Self {
        self.skill = skill;
        self.disposition = disposition;
        self
    }
}

/// One action turning one property into another — knapping a flint into
/// something `Sharp`, twisting bast into something `Fibrous`.
///
/// One action, not a recipe tree: §3.5 allows the mind exactly one step of
/// inference, and this type is what makes that limit structural rather than a
/// rule somebody has to remember.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OneStep {
    /// The property consumed.
    pub from: Tag,
    /// The property produced.
    pub into: Tag,
}

impl OneStep {
    /// `from` becomes `into` in a single action.
    pub const fn new(from: Tag, into: Tag) -> Self {
        Self { from, into }
    }
}

/// What a settler is carrying and what they know how to make from it in one
/// action.
///
/// This is deliberately not an inventory: the mind never asks *which log*, it
/// asks whether anything to hand is `Hard`, the same way a stake asks for a
/// property and not a material.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AtHand {
    holding: Vec<Tag>,
    steps: Vec<OneStep>,
}

impl AtHand {
    /// Carrying `holding`, and able to perform every step in `steps`.
    pub fn new(
        holding: impl IntoIterator<Item = Tag>,
        steps: impl IntoIterator<Item = OneStep>,
    ) -> Self {
        Self {
            holding: holding.into_iter().collect(),
            steps: steps.into_iter().collect(),
        }
    }

    /// Carrying `holding` and knowing no way to make anything else.
    pub fn holding(holding: impl IntoIterator<Item = Tag>) -> Self {
        Self::new(holding, [])
    }

    /// Whether something already in hand has this property.
    pub fn has(&self, tag: Tag) -> bool {
        self.holding.contains(&tag)
    }

    /// Whether `tag` is one action away: the settler does not have it, but
    /// something they *are* holding turns into it in a single step.
    ///
    /// Two steps is not one step, by construction — the output of a step is
    /// never fed back in.
    pub fn one_step_from(&self, tag: Tag) -> bool {
        !self.has(tag)
            && self
                .steps
                .iter()
                .any(|step| step.into == tag && self.has(step.from))
    }

    /// Put `tag` in hand — what performing a [`OneStep`] amounts to, as far
    /// as the mind is concerned.
    pub fn take(&mut self, tag: Tag) {
        if !self.has(tag) {
            self.holding.push(tag);
        }
    }

    /// Drop `tag`, if it was in hand.
    pub fn drop(&mut self, tag: Tag) {
        self.holding.retain(|held| *held != tag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    fn hand() -> AtHand {
        AtHand::new(
            [Tag::Hard],
            [
                OneStep::new(Tag::Hard, Tag::Sharp),
                OneStep::new(Tag::Sharp, Tag::Malleable),
            ],
        )
    }

    #[test]
    fn what_is_already_in_hand_is_not_one_step_away() {
        let hand = hand();
        assert!(hand.has(Tag::Hard));
        assert!(
            !hand.one_step_from(Tag::Hard),
            "there is nothing to infer about a thing you are already holding"
        );
    }

    #[test]
    fn one_step_is_one_step_and_two_steps_is_not() {
        let hand = hand();
        assert!(
            hand.one_step_from(Tag::Sharp),
            "Hard in hand, Hard -> Sharp"
        );
        assert!(
            !hand.one_step_from(Tag::Malleable),
            "Malleable is Hard -> Sharp -> Malleable, which is two actions"
        );
    }

    #[test]
    fn a_step_whose_input_is_not_in_hand_infers_nothing() {
        let hand = AtHand::new([Tag::Warm], [OneStep::new(Tag::Hard, Tag::Sharp)]);
        assert!(!hand.one_step_from(Tag::Sharp));
    }

    #[test]
    fn taking_and_dropping_move_the_line_between_have_and_could_make() {
        let mut hand = hand();
        assert!(hand.one_step_from(Tag::Sharp));
        hand.take(Tag::Sharp);
        assert!(hand.has(Tag::Sharp));
        assert!(!hand.one_step_from(Tag::Sharp), "it is in hand now");
        // Malleable was two steps away; with Sharp in hand it is one.
        assert!(hand.one_step_from(Tag::Malleable));

        hand.drop(Tag::Sharp);
        assert!(!hand.has(Tag::Sharp));
        assert!(hand.one_step_from(Tag::Sharp));
    }

    #[test]
    fn taking_something_twice_does_not_duplicate_it() {
        let mut hand = AtHand::holding([Tag::Hard]);
        hand.take(Tag::Hard);
        hand.drop(Tag::Hard);
        assert!(!hand.has(Tag::Hard));
    }

    #[test]
    fn a_work_candidate_is_built_from_the_work_and_the_settler_looking_at_it() {
        let mut world = World::new();
        let stake = world.spawn();
        let work = WorkCandidate::new(stake, 0.8, 12.0)
            .wanting(Tag::Hard)
            .by(0.5, 0.9);
        assert_eq!(work.target, stake);
        assert_eq!(work.wants, Some(Tag::Hard));
        assert_eq!(work.skill, 0.5);
        assert_eq!(work.disposition, 0.9);
    }
}
