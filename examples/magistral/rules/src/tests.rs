use std::sync::Arc;

use crate::ai::{play, Greedy, Quick, Random};
use crate::*;

fn new(seed: u64) -> State {
    State::new(Arc::new(Config::default()), seed)
}

#[test]
fn the_shipped_rules_parse() {
    let cfg = Config::default();
    assert_eq!(cfg.start_deck.len(), 10);
    for card in Card::ALL {
        assert!(
            cfg.cards.contains_key(&card),
            "{card:?} has no numbers in rules.ron"
        );
    }
}

#[test]
fn one_seed_is_one_game() {
    let a = play(new(7), &mut Random(Rng::new(1)));
    let b = play(new(7), &mut Random(Rng::new(1)));
    assert_eq!(a.score(), b.score());
    assert_eq!(a.hand, b.hand);
}

#[test]
fn a_map_has_its_start_and_its_places() {
    for seed in 0..200 {
        let s = new(seed);
        assert_eq!(s.stations().count(), 1, "seed {seed}");
        let sites = s.cells.iter().filter(|c| c.site.is_some()).count();
        assert_eq!(sites, 7, "seed {seed}");
        assert_eq!(s.hand.len(), 5);
        assert!(s.nearest_unreached().is_some());
    }
}

#[test]
fn every_listed_move_can_be_made_and_runs_end() {
    for seed in 0..50 {
        let mut s = new(seed);
        let mut rng = Rng::new(seed);
        let mut steps = 0;
        while !s.over() {
            let moves = s.moves();
            assert!(!moves.is_empty(), "no move in {:?}", s.phase);
            let before = s.hand.len() + s.draw.len() + s.discard.len();
            let mv = moves[rng.below(moves.len())];
            s.apply(mv);
            let after = s.hand.len() + s.draw.len() + s.discard.len();
            if let Move::Play {
                card: Card::Writeoff,
                ..
            } = mv
            {
                assert_eq!(after + 2, before);
            }
            steps += 1;
            assert!(steps < 10_000);
        }
    }
}

#[test]
fn track_station_and_a_train_earn() {
    // Greedy on its own must get somewhere: stations, a train, coins.
    let mut got = 0;
    for seed in 0..20 {
        let s = play(new(seed), &mut Greedy);
        if s.earned > 0 {
            got += 1;
        }
    }
    assert!(got >= 15, "greedy earned in only {got} of 20 runs");
}

#[test]
fn the_quick_policy_finishes_runs() {
    for seed in 0..20 {
        let s = play(new(seed), &mut Quick(Rng::new(seed)));
        assert!(s.over());
    }
}
