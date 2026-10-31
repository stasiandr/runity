//! The players of the depth ladder, weakest first:
//!
//! * `Random` — any legal move.
//! * `Greedy` — the move that brings the most coins this turn and the next;
//!   with nothing to earn, the move that gets the network further. It never
//!   pays now for later.
//! * `Search` — Monte Carlo tree search over what it cannot see (the order
//!   of the deck, the contracts and markets to come): each iteration deals a
//!   fresh guess of the hidden cards, walks the tree, then plays on with a
//!   quick policy to the end of the next stage (single-observer ISMCTS,
//!   Cowling, Powley, Whitehouse 2012). More iterations, stronger play.
//!
//! None of them peeks: they see what the screen shows.

use crate::{Card, Move, Phase, Rng, State};

pub trait Player: Send {
    fn name(&self) -> String;
    fn choose(&mut self, state: &State) -> Move;
}

/// Play a whole run; the final state.
pub fn play(mut state: State, player: &mut dyn Player) -> State {
    let mut guard = 0;
    while !state.over() && guard < 100_000 {
        let mv = player.choose(&state);
        state.apply(mv);
        guard += 1;
    }
    state
}

// ----- random --------------------------------------------------------------

pub struct Random(pub Rng);

impl Player for Random {
    fn name(&self) -> String {
        "random".into()
    }

    fn choose(&mut self, state: &State) -> Move {
        let moves = state.moves();
        moves[self.0.below(moves.len())]
    }
}

// ----- greedy --------------------------------------------------------------

pub struct Greedy;

/// Coins two turns from now if nothing more were played.
fn projected(state: &State) -> i32 {
    let mut s = state.clone();
    s.idle_turns(2);
    s.money - s.debt
}

/// How far the network has got: stations and trains, and how near the
/// nearest place not yet reached.
fn progress(state: &State) -> i32 {
    let stations = state.stations().count() as i32;
    let trains = state.trains.len() as i32;
    let near = state.nearest_unreached().map_or(0, |d| 8 - d.min(8) as i32);
    stations * 20 + trains * 12 + near
}

/// What a card looks worth at a glance, for a player who looks no further.
fn glance(card: Card) -> i32 {
    match card {
        Card::Subsidy => 9,
        Card::Loco | Card::Express | Card::Freight => 6,
        Card::Mainline => 5,
        Card::Station => 4,
        Card::Overtime | Card::Survey => 3,
        Card::Timetable => 2,
        _ => 0,
    }
}

impl Player for Greedy {
    fn name(&self) -> String {
        "greedy".into()
    }

    fn choose(&mut self, state: &State) -> Move {
        match &state.phase {
            Phase::Pick(offers) => {
                let best = offers.iter().copied().max_by_key(|&c| glance(c));
                return Move::Pick(best);
            }
            Phase::Remove => return Move::Remove(None),
            _ => {}
        }
        let base = (projected(state), progress(state));
        let mut best = (Move::EndTurn, (0, 0));
        for mv in state.moves() {
            if mv == Move::EndTurn {
                continue;
            }
            let mut s = state.clone();
            s.apply(mv);
            let gain = (projected(&s) - base.0, progress(&s) - base.1);
            if gain.0 < 0 || (gain.0 == 0 && gain.1 <= 0) {
                continue;
            }
            if gain > best.1 {
                best = (mv, gain);
            }
        }
        best.0
    }
}

// ----- the quick policy searches play on with ------------------------------

/// Quick and rough: favours stations, a train on every new route, track
/// toward places not reached, and ends the turn now and then. No look ahead.
pub struct Quick(pub Rng);

impl Quick {
    pub fn pick(rng: &mut Rng, state: &State) -> Move {
        let moves = state.moves();
        let mut weights = Vec::with_capacity(moves.len());
        for &mv in &moves {
            let w = match mv {
                Move::EndTurn => 1.0,
                Move::Pick(None) => 1.0,
                Move::Pick(Some(_)) => 1.0,
                Move::Remove(None) => 4.0,
                Move::Remove(Some(Card::Path)) => 1.0,
                Move::Remove(Some(_)) => 0.2,
                Move::Play { card, a, b, .. } => match card {
                    Card::Station => 8.0,
                    Card::Loco | Card::Freight | Card::Express => {
                        let served = state
                            .trains
                            .iter()
                            .any(|t| (t.a, t.b) == (a, b) || (t.a, t.b) == (b, a));
                        if served {
                            0.3
                        } else {
                            6.0
                        }
                    }
                    Card::Path | Card::Mainline | Card::Bridge | Card::Tunnel => 3.0,
                    Card::Survey | Card::Overtime => 5.0,
                    Card::Subsidy => 1.0,
                    Card::Shares => 1.5,
                    Card::Timetable => 2.0,
                    Card::Warehouse => 0.7,
                    Card::Writeoff => 0.5,
                    Card::Dispatcher => 0.3,
                },
            };
            weights.push(w);
        }
        let total: f64 = weights.iter().sum();
        let mut roll = rng.unit() * total;
        for (mv, w) in moves.iter().zip(&weights) {
            roll -= w;
            if roll <= 0.0 {
                return *mv;
            }
        }
        moves[moves.len() - 1]
    }
}

impl Player for Quick {
    fn name(&self) -> String {
        "quick".into()
    }

    fn choose(&mut self, state: &State) -> Move {
        Quick::pick(&mut self.0, state)
    }
}

// ----- search --------------------------------------------------------------

pub struct Search {
    pub iterations: usize,
    pub rng: Rng,
    /// How wide the tree looks before it looks deep.
    pub explore: f64,
}

impl Search {
    pub fn new(iterations: usize, seed: u64) -> Search {
        Search {
            iterations,
            rng: Rng::new(seed),
            explore: 0.7,
        }
    }
}

struct Node {
    mv: Move,
    children: Vec<usize>,
    visits: f64,
    avail: f64,
    total: f64,
}

/// How good a state is for the player, about one unit a stage paid.
fn value(state: &State, root_stage: u8) -> f64 {
    let stage = state.stage as f64;
    let earned = state.earned as f64 / 400.0;
    if state.over() {
        return stage + earned;
    }
    // Still running: how ready the till is for what is due next.
    let due = state.payment_due().max(1) as f64;
    let ready = (state.money as f64 / due).min(1.5);
    let _ = root_stage;
    stage + earned + 0.5 * ready
}

impl Player for Search {
    fn name(&self) -> String {
        format!("search {}", self.iterations)
    }

    fn choose(&mut self, state: &State) -> Move {
        let moves = state.moves();
        if moves.len() == 1 {
            return moves[0];
        }
        let mut nodes = vec![Node {
            mv: Move::EndTurn,
            children: Vec::new(),
            visits: 0.0,
            avail: 0.0,
            total: 0.0,
        }];
        let horizon = state.stage + 2;
        for _ in 0..self.iterations {
            let mut s = state.clone();
            s.determinize(&mut self.rng);
            let mut path = vec![0usize];
            let mut node = 0usize;
            // Select and expand.
            loop {
                if s.over() {
                    break;
                }
                let legal = s.moves();
                let untried: Vec<Move> = legal
                    .iter()
                    .copied()
                    .filter(|mv| !nodes[node].children.iter().any(|&c| nodes[c].mv == *mv))
                    .collect();
                if !untried.is_empty() {
                    let mv = untried[self.rng.below(untried.len())];
                    nodes.push(Node {
                        mv,
                        children: Vec::new(),
                        visits: 0.0,
                        avail: 1.0,
                        total: 0.0,
                    });
                    let child = nodes.len() - 1;
                    nodes[node].children.push(child);
                    // The others legal here were available too.
                    for c in nodes[node].children.clone() {
                        if c != child && legal.contains(&nodes[c].mv) {
                            nodes[c].avail += 1.0;
                        }
                    }
                    s.apply(mv);
                    path.push(child);
                    break;
                }
                let mut best = None;
                let mut best_score = f64::MIN;
                for c in nodes[node].children.clone() {
                    if !legal.contains(&nodes[c].mv) {
                        continue;
                    }
                    nodes[c].avail += 1.0;
                    let n = &nodes[c];
                    let score = n.total / n.visits.max(1e-9)
                        + self.explore * (n.avail.ln() / n.visits.max(1e-9)).sqrt();
                    if score > best_score {
                        best_score = score;
                        best = Some(c);
                    }
                }
                let Some(c) = best else { break };
                s.apply(nodes[c].mv);
                path.push(c);
                node = c;
            }
            // Play on, quickly, to the end of the next stage.
            let mut guard = 0;
            while !s.over() && s.stage < horizon && guard < 2000 {
                let mv = Quick::pick(&mut self.rng, &s);
                s.apply(mv);
                guard += 1;
            }
            let v = value(&s, state.stage);
            for &n in &path {
                nodes[n].visits += 1.0;
                nodes[n].total += v;
            }
        }
        nodes[0]
            .children
            .iter()
            .filter(|&&c| moves.contains(&nodes[c].mv))
            .max_by(|&&a, &&b| nodes[a].visits.partial_cmp(&nodes[b].visits).unwrap())
            .map(|&c| nodes[c].mv)
            .unwrap_or(Move::EndTurn)
    }
}
