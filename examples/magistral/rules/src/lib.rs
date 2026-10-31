//! Magistral: a railway deckbuilder. You lay track and run trains with
//! cards; the trains earn on their own; the bank wants a payment at the end
//! of every stage.
//!
//! Everything here is a plain value: `State` clones cheaply, `moves` lists
//! what may be done, `apply` does one. One seed is one game — the map, the
//! shuffles, the contracts and the market all come from the state's own
//! generator — so two players on one seed meet the same luck.

pub mod ai;
pub mod config;
pub mod map;
pub mod rng;

use std::collections::VecDeque;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use config::Config;
pub use rng::Rng;

pub const W: usize = 8;
pub const H: usize = 8;
pub const CELLS: usize = W * H;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Terrain {
    Plain,
    Hill,
    Water,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Site {
    City,
    Coal,
    Ore,
    Farm,
    Factory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cargo {
    Coal = 0,
    Ore = 1,
    Grain = 2,
    Steel = 3,
}

pub const CARGOS: [Cargo; 4] = [Cargo::Coal, Cargo::Ore, Cargo::Grain, Cargo::Steel];

impl Site {
    /// What a station here hands to a train.
    pub fn provides(self) -> &'static [Cargo] {
        match self {
            Site::Coal => &[Cargo::Coal],
            Site::Ore => &[Cargo::Ore],
            Site::Farm => &[Cargo::Grain],
            Site::Factory => &[Cargo::Steel],
            Site::City => &[],
        }
    }

    /// What a station here takes from a train, for coins.
    pub fn accepts(self, cargo: Cargo) -> bool {
        match self {
            Site::City => matches!(cargo, Cargo::Coal | Cargo::Grain | Cargo::Steel),
            Site::Factory => matches!(cargo, Cargo::Coal | Cargo::Ore),
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Card {
    Path,
    Mainline,
    Bridge,
    Tunnel,
    Station,
    Loco,
    Freight,
    Express,
    Survey,
    Overtime,
    Dispatcher,
    Warehouse,
    Subsidy,
    Shares,
    Timetable,
    Writeoff,
}

impl Card {
    pub const ALL: [Card; 16] = [
        Card::Path,
        Card::Mainline,
        Card::Bridge,
        Card::Tunnel,
        Card::Station,
        Card::Loco,
        Card::Freight,
        Card::Express,
        Card::Survey,
        Card::Overtime,
        Card::Dispatcher,
        Card::Warehouse,
        Card::Subsidy,
        Card::Shares,
        Card::Timetable,
        Card::Writeoff,
    ];

    pub fn is_track(self) -> bool {
        matches!(
            self,
            Card::Path | Card::Mainline | Card::Bridge | Card::Tunnel
        )
    }

    pub fn is_train(self) -> bool {
        matches!(self, Card::Loco | Card::Freight | Card::Express)
    }

    /// The localisation key of the card's name, and of its line of rules (`.text`).
    pub fn key(self) -> &'static str {
        match self {
            Card::Path => "card.path",
            Card::Mainline => "card.mainline",
            Card::Bridge => "card.bridge",
            Card::Tunnel => "card.tunnel",
            Card::Station => "card.station",
            Card::Loco => "card.loco",
            Card::Freight => "card.freight",
            Card::Express => "card.express",
            Card::Survey => "card.survey",
            Card::Overtime => "card.overtime",
            Card::Dispatcher => "card.dispatcher",
            Card::Warehouse => "card.warehouse",
            Card::Subsidy => "card.subsidy",
            Card::Shares => "card.shares",
            Card::Timetable => "card.timetable",
            Card::Writeoff => "card.writeoff",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cell {
    pub terrain: Terrain,
    pub site: Option<Site>,
    pub track: bool,
    pub station: bool,
    /// Goods waiting at the station, by `Cargo as usize`.
    pub stock: [u8; 4],
    pub warehouse: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Train {
    pub kind: Card,
    pub speed: u8,
    pub capacity: u8,
    /// The two stations, and the cells between them from `a` to `b`.
    pub a: u8,
    pub b: u8,
    pub path: Vec<u8>,
    pub at: usize,
    pub forward: bool,
    pub cargo: Option<(Cargo, u8)>,
}

impl Train {
    pub fn cell(&self) -> u8 {
        self.path[self.at]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Contract {
    pub cargo: Cargo,
    pub city: u8,
    pub amount: u8,
    pub done: u8,
    pub turns: u8,
    pub reward: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Play,
    /// Between stages: take one of the offers, or none.
    Pick(Vec<Card>),
    /// Then throw one card out of the deck for coins, or none.
    Remove,
    Over,
}

/// One thing a player does. `Play` names a card kind in the hand, not a
/// slot: two Paths in hand are one choice, not two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Move {
    EndTurn,
    /// `a`, `b`, `c` mean what the card needs: a target cell, two stations,
    /// a train and its new route.
    Play {
        card: Card,
        a: u8,
        b: u8,
        c: u8,
    },
    Pick(Option<Card>),
    Remove(Option<Card>),
}

/// Something that happened, for the screen to show and the log to keep.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    Delivered {
        cell: u8,
        cargo: Cargo,
        units: u8,
        coins: i32,
    },
    ContractDone {
        reward: i32,
    },
    ContractLost,
    Paid {
        coins: i32,
    },
    Bankrupt {
        owed: i32,
    },
    Won,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    #[serde(skip)]
    pub cfg: Arc<Config>,
    pub cells: Vec<Cell>,
    pub trains: Vec<Train>,
    pub draw: Vec<Card>,
    pub hand: Vec<Card>,
    pub discard: Vec<Card>,
    pub energy: u8,
    pub money: i32,
    /// Every coin ever earned: the tie-break of the score.
    pub earned: i32,
    /// Turns played in this stage, and stages paid for.
    pub turn: u8,
    pub stage: u8,
    pub debt: i32,
    pub shares: i32,
    /// The active contract first, then the ones coming.
    pub contracts: VecDeque<Contract>,
    pub phase: Phase,
    pub won: bool,
    #[serde(skip)]
    pub events: Vec<Event>,
    #[serde(skip)]
    pub rng: Rng,
}

pub fn xy(cell: u8) -> (usize, usize) {
    (cell as usize % W, cell as usize / W)
}

pub fn neighbours(cell: u8) -> impl Iterator<Item = u8> {
    let (x, y) = xy(cell);
    let mut out = [None; 4];
    if x > 0 {
        out[0] = Some(cell - 1);
    }
    if x + 1 < W {
        out[1] = Some(cell + 1);
    }
    if y > 0 {
        out[2] = Some(cell - W as u8);
    }
    if y + 1 < H {
        out[3] = Some(cell + W as u8);
    }
    out.into_iter().flatten()
}

impl State {
    /// A new run on `seed`: its map, its shuffled deck, its first hand.
    pub fn new(cfg: Arc<Config>, seed: u64) -> State {
        let mut rng = Rng::new(seed);
        let (cells, _start) = map::generate(&mut rng);
        let mut draw = cfg.start_deck.clone();
        rng.shuffle(&mut draw);
        let mut state = State {
            money: cfg.start_money,
            cfg,
            cells,
            trains: Vec::new(),
            draw,
            hand: Vec::new(),
            discard: Vec::new(),
            energy: 0,
            earned: 0,
            turn: 0,
            stage: 0,
            debt: 0,
            shares: 0,
            contracts: VecDeque::new(),
            phase: Phase::Play,
            won: false,
            events: Vec::new(),
            rng,
        };
        while state.contracts.len() < 3 {
            let c = state.new_contract();
            state.contracts.push_back(c);
        }
        state.start_turn();
        state
    }

    pub fn over(&self) -> bool {
        self.phase == Phase::Over
    }

    pub fn stages(&self) -> usize {
        self.cfg.payments.len()
    }

    /// The run's score: stages paid for first, coins earned second.
    pub fn score(&self) -> i64 {
        self.stage as i64 * 1000 + self.earned as i64
    }

    pub fn payment_due(&self) -> i32 {
        self.cfg
            .payments
            .get(self.stage as usize)
            .copied()
            .unwrap_or(0)
            + self.debt
    }

    pub fn turns_left_in_stage(&self) -> u8 {
        self.cfg.turns_per_stage - self.turn
    }

    /// Hide what a player cannot know: the order of the draw pile and the
    /// luck to come. A search plays many of these.
    pub fn determinize(&mut self, rng: &mut Rng) {
        rng.shuffle(&mut self.draw);
        self.rng = Rng::new(rng.next_u64());
    }

    // ----- turns -----------------------------------------------------------

    fn start_turn(&mut self) {
        self.energy = self.cfg.energy;
        let hand = self.cfg.hand as usize;
        self.draw_cards(hand.saturating_sub(self.hand.len()));
    }

    fn draw_cards(&mut self, n: usize) {
        for _ in 0..n {
            if self.draw.is_empty() {
                if self.discard.is_empty() {
                    return;
                }
                std::mem::swap(&mut self.draw, &mut self.discard);
                let mut draw = std::mem::take(&mut self.draw);
                self.rng.shuffle(&mut draw);
                self.draw = draw;
            }
            if let Some(card) = self.draw.pop() {
                self.hand.push(card);
            }
        }
    }

    fn end_turn(&mut self) {
        self.discard.append(&mut self.hand);
        self.run_trains();
        self.produce();
        let income = self.shares * self.cfg.shares_income;
        self.money += income;
        self.earned += income;
        self.tick_contract();
        self.turn += 1;
        if self.turn >= self.cfg.turns_per_stage {
            self.end_stage();
        } else {
            self.start_turn();
        }
    }

    fn end_stage(&mut self) {
        let due = self.payment_due();
        if self.money < due {
            self.events.push(Event::Bankrupt { owed: due });
            self.phase = Phase::Over;
            return;
        }
        self.money -= due;
        self.debt = 0;
        self.events.push(Event::Paid { coins: due });
        self.stage += 1;
        self.turn = 0;
        if self.stage as usize >= self.stages() {
            self.won = true;
            self.events.push(Event::Won);
            self.phase = Phase::Over;
            return;
        }
        // The whole deck into one pile for the market.
        let mut all = std::mem::take(&mut self.discard);
        all.append(&mut self.hand);
        all.append(&mut self.draw);
        self.draw = all;
        let mut offers = Vec::new();
        let pool = self.cfg.pool.clone();
        while offers.len() < 3 && offers.len() < pool.len() {
            let card = pool[self.rng.below(pool.len())];
            if !offers.contains(&card) {
                offers.push(card);
            }
        }
        self.phase = Phase::Pick(offers);
    }

    fn begin_stage(&mut self) {
        let mut draw = std::mem::take(&mut self.draw);
        self.rng.shuffle(&mut draw);
        self.draw = draw;
        self.phase = Phase::Play;
        self.start_turn();
    }

    /// Let `turns` turns pass with no card played: trains, goods, shares.
    /// What a player sees coming from what is already built.
    pub fn idle_turns(&mut self, turns: u8) {
        for _ in 0..turns {
            self.run_trains();
            self.produce();
            self.money += self.shares * self.cfg.shares_income;
        }
    }

    /// Cells of plain track from the network to the nearest place it has no
    /// station at.
    pub fn nearest_unreached(&self) -> Option<usize> {
        (0..CELLS as u8)
            .filter(|&c| self.cells[c as usize].site.is_some() && !self.cells[c as usize].station)
            .filter_map(|c| {
                if self.touches_network(c) {
                    Some(0)
                } else {
                    self.shortest_from_network(c, Card::Path).map(|p| p.len())
                }
            })
            .min()
    }

    // ----- the world -------------------------------------------------------

    fn run_trains(&mut self) {
        for t in 0..self.trains.len() {
            let speed = self.trains[t].speed;
            self.move_train(t, speed);
        }
    }

    fn move_train(&mut self, t: usize, steps: u8) {
        for _ in 0..steps {
            let train = &mut self.trains[t];
            if train.path.len() < 2 {
                return;
            }
            if train.forward {
                train.at += 1;
            } else {
                train.at -= 1;
            }
            let end = train.at == 0 || train.at == train.path.len() - 1;
            if end {
                train.forward = train.at == 0;
                let cell = train.cell();
                self.arrive(t, cell);
                return;
            }
        }
    }

    /// A train at a station: sells what this station takes, then loads
    /// what the other end takes.
    fn arrive(&mut self, t: usize, cell: u8) {
        let site = match self.cells[cell as usize].site {
            Some(site) => site,
            None => return,
        };
        let length = self.trains[t].path.len() as i32 - 1;
        if let Some((cargo, units)) = self.trains[t].cargo {
            if site.accepts(cargo) {
                self.trains[t].cargo = None;
                let each =
                    self.cfg.price[cargo as usize] + length / self.cfg.distance_bonus_every.max(1);
                let coins = each * units as i32;
                self.money += coins;
                self.earned += coins;
                self.events.push(Event::Delivered {
                    cell,
                    cargo,
                    units,
                    coins,
                });
                if site == Site::Factory {
                    let stock = &mut self.cells[cell as usize].stock[cargo as usize];
                    *stock = stock.saturating_add(units);
                }
                self.deliver_to_contract(cell, cargo, units);
            }
        }
        if self.trains[t].cargo.is_none() {
            let other = if self.trains[t].a == cell {
                self.trains[t].b
            } else {
                self.trains[t].a
            };
            let wants = self.cells[other as usize].site;
            let capacity = self.trains[t].capacity;
            for &cargo in site.provides() {
                if wants.is_some_and(|w| w.accepts(cargo)) {
                    let stock = &mut self.cells[cell as usize].stock[cargo as usize];
                    let units = (*stock).min(capacity);
                    if units > 0 {
                        *stock -= units;
                        self.trains[t].cargo = Some((cargo, units));
                        break;
                    }
                }
            }
        }
    }

    fn produce(&mut self) {
        for cell in self.cells.iter_mut() {
            if !cell.station {
                continue;
            }
            let cap = if cell.warehouse {
                self.cfg.warehouse_cap
            } else {
                self.cfg.stock_cap
            };
            match cell.site {
                Some(Site::Coal) => cell.stock[0] = (cell.stock[0] + 1).min(cap),
                Some(Site::Ore) => cell.stock[1] = (cell.stock[1] + 1).min(cap),
                Some(Site::Farm) => cell.stock[2] = (cell.stock[2] + 1).min(cap),
                Some(Site::Factory) => {
                    let n = cell.stock[0].min(cell.stock[1]).min(self.cfg.factory_rate);
                    cell.stock[0] -= n;
                    cell.stock[1] -= n;
                    cell.stock[3] = (cell.stock[3] + n).min(cap);
                }
                _ => {}
            }
        }
    }

    fn deliver_to_contract(&mut self, cell: u8, cargo: Cargo, units: u8) {
        let Some(c) = self.contracts.front_mut() else {
            return;
        };
        if c.city != cell || c.cargo != cargo {
            return;
        }
        c.done = c.done.saturating_add(units);
        if c.done >= c.amount {
            let reward = c.reward;
            self.money += reward;
            self.earned += reward;
            self.events.push(Event::ContractDone { reward });
            self.next_contract();
        }
    }

    fn tick_contract(&mut self) {
        let Some(c) = self.contracts.front_mut() else {
            return;
        };
        c.turns = c.turns.saturating_sub(1);
        if c.turns == 0 {
            self.events.push(Event::ContractLost);
            self.next_contract();
        }
    }

    fn next_contract(&mut self) {
        self.contracts.pop_front();
        let c = self.new_contract();
        self.contracts.push_back(c);
    }

    fn new_contract(&mut self) -> Contract {
        let cities: Vec<u8> = (0..CELLS as u8)
            .filter(|&c| self.cells[c as usize].site == Some(Site::City))
            .collect();
        let city = cities[self.rng.below(cities.len())];
        let roll = self.rng.below(10);
        let cargo = match roll {
            0..=3 => Cargo::Coal,
            4..=6 => Cargo::Grain,
            _ => Cargo::Steel,
        };
        let amount = 2 + self.rng.below(4) as u8;
        let turns = 6 + self.rng.below(5) as u8;
        let reward = amount as i32 * (self.cfg.price[cargo as usize] + 2) + 2;
        Contract {
            cargo,
            city,
            amount,
            done: 0,
            turns,
            reward,
        }
    }

    // ----- track -----------------------------------------------------------

    pub fn network_empty(&self) -> bool {
        !self.cells.iter().any(|c| c.track)
    }

    fn passable(&self, cell: u8, card: Card) -> bool {
        let c = &self.cells[cell as usize];
        if c.site.is_some() {
            return false;
        }
        match c.terrain {
            Terrain::Plain => true,
            Terrain::Water => card == Card::Bridge,
            Terrain::Hill => card == Card::Tunnel,
        }
    }

    /// The cells a track card lays toward `target`, nearest the network
    /// first: the shortest way from the network to a cell beside the target
    /// over what this card may cross, its first `cells` new cells.
    pub fn track_route(&self, card: Card, target: u8) -> Option<Vec<u8>> {
        if self.cells[target as usize].site.is_none() || self.touches_network(target) {
            return None;
        }
        let path = self.shortest_from_network(target, card)?;
        let cells = self.cfg.card(card).cells as usize;
        let new: Vec<u8> = path
            .into_iter()
            .filter(|&c| !self.cells[c as usize].track)
            .take(cells)
            .collect();
        if new.is_empty() {
            None
        } else {
            Some(new)
        }
    }

    fn touches_network(&self, cell: u8) -> bool {
        self.cells[cell as usize].track || neighbours(cell).any(|n| self.cells[n as usize].track)
    }

    /// Breadth first from every track cell to a cell beside `target`;
    /// the path without its start (which is track already).
    fn shortest_from_network(&self, target: u8, card: Card) -> Option<Vec<u8>> {
        let mut prev = [u8::MAX; CELLS];
        let mut seen = [false; CELLS];
        let mut queue = VecDeque::new();
        for c in 0..CELLS as u8 {
            if self.cells[c as usize].track {
                seen[c as usize] = true;
                queue.push_back(c);
            }
        }
        while let Some(c) = queue.pop_front() {
            if neighbours(c).any(|n| n == target) && !self.cells[c as usize].track {
                let mut path = vec![c];
                let mut at = c;
                while prev[at as usize] != u8::MAX {
                    at = prev[at as usize];
                    if self.cells[at as usize].track {
                        break;
                    }
                    path.push(at);
                }
                path.reverse();
                return Some(path);
            }
            for n in neighbours(c) {
                if !seen[n as usize] && self.passable(n, card) {
                    seen[n as usize] = true;
                    prev[n as usize] = c;
                    queue.push_back(n);
                }
            }
        }
        None
    }

    /// The way along the track from station `a` to station `b`.
    pub fn rail_path(&self, a: u8, b: u8) -> Option<Vec<u8>> {
        let mut prev = [u8::MAX; CELLS];
        let mut seen = [false; CELLS];
        let mut queue = VecDeque::from([a]);
        seen[a as usize] = true;
        while let Some(c) = queue.pop_front() {
            if c == b {
                let mut path = vec![b];
                let mut at = b;
                while at != a {
                    at = prev[at as usize];
                    path.push(at);
                }
                path.reverse();
                return Some(path);
            }
            for n in neighbours(c) {
                let cell = &self.cells[n as usize];
                // Trains pass through track, and stop only at their own ends.
                let open = cell.track && (cell.site.is_none() || n == b);
                if !seen[n as usize] && open {
                    seen[n as usize] = true;
                    prev[n as usize] = c;
                    queue.push_back(n);
                }
            }
        }
        None
    }

    pub fn stations(&self) -> impl Iterator<Item = u8> + '_ {
        (0..CELLS as u8).filter(|&c| self.cells[c as usize].station)
    }

    /// Does anything go from one end of this route to the other?
    pub fn route_carries(&self, a: u8, b: u8) -> bool {
        let (Some(sa), Some(sb)) = (self.cells[a as usize].site, self.cells[b as usize].site)
        else {
            return false;
        };
        sa.provides().iter().any(|&c| sb.accepts(c)) || sb.provides().iter().any(|&c| sa.accepts(c))
    }

    // ----- moves -----------------------------------------------------------

    pub fn moves(&self) -> Vec<Move> {
        let mut out = Vec::new();
        match &self.phase {
            Phase::Over => {}
            Phase::Pick(offers) => {
                out.push(Move::Pick(None));
                out.extend(offers.iter().map(|&c| Move::Pick(Some(c))));
            }
            Phase::Remove => {
                out.push(Move::Remove(None));
                if self.money >= self.cfg.remove_cost {
                    let mut kinds = self.draw.clone();
                    kinds.sort();
                    kinds.dedup();
                    out.extend(kinds.into_iter().map(|c| Move::Remove(Some(c))));
                }
            }
            Phase::Play => {
                out.push(Move::EndTurn);
                let mut kinds = self.hand.clone();
                kinds.sort();
                kinds.dedup();
                for card in kinds {
                    if self.cfg.card(card).cost > self.energy {
                        continue;
                    }
                    self.card_moves(card, &mut out);
                }
            }
        }
        out
    }

    fn card_moves(&self, card: Card, out: &mut Vec<Move>) {
        let play = |a: u8, b: u8, c: u8| Move::Play { card, a, b, c };
        match card {
            Card::Path | Card::Mainline | Card::Bridge | Card::Tunnel => {
                for target in 0..CELLS as u8 {
                    if self.cells[target as usize].site.is_none()
                        || self.cells[target as usize].station
                    {
                        continue;
                    }
                    let Some(route) = self.track_route(card, target) else {
                        continue;
                    };
                    if matches!(card, Card::Bridge | Card::Tunnel) {
                        // Only where a plain path would not do as well.
                        let special = Terrain::from_card(card);
                        if !route
                            .iter()
                            .any(|&c| self.cells[c as usize].terrain == special)
                        {
                            continue;
                        }
                    }
                    out.push(play(target, 0, 0));
                }
            }
            Card::Station => {
                for c in 0..CELLS as u8 {
                    let cell = &self.cells[c as usize];
                    if cell.site.is_some() && !cell.station && self.touches_network(c) {
                        out.push(play(c, 0, 0));
                    }
                }
            }
            Card::Loco | Card::Freight | Card::Express => {
                if self.trains.len() >= self.cfg.max_trains as usize {
                    return;
                }
                for (a, b) in self.routes() {
                    out.push(play(a, b, 0));
                }
            }
            Card::Dispatcher => {
                let routes = self.routes();
                for (t, train) in self.trains.iter().enumerate() {
                    for &(a, b) in &routes {
                        if (a, b) != (train.a, train.b) && (b, a) != (train.a, train.b) {
                            out.push(play(t as u8, a, b));
                        }
                    }
                }
            }
            Card::Warehouse => {
                for c in self.stations() {
                    let cell = &self.cells[c as usize];
                    if !cell.warehouse && cell.site.is_some_and(|s| !s.provides().is_empty()) {
                        out.push(play(c, 0, 0));
                    }
                }
            }
            Card::Shares => {
                if self.money >= self.cfg.shares_price {
                    out.push(play(0, 0, 0));
                }
            }
            Card::Timetable => {
                if !self.trains.is_empty() {
                    out.push(play(0, 0, 0));
                }
            }
            Card::Writeoff => {
                let mut kinds: Vec<Card> = self
                    .hand
                    .iter()
                    .copied()
                    .filter(|&c| c != Card::Writeoff)
                    .collect();
                kinds.sort();
                kinds.dedup();
                let mut has_other = false;
                for k in kinds {
                    has_other = true;
                    out.push(play(k as u8, 0, 0));
                }
                let writeoffs = self.hand.iter().filter(|&&c| c == Card::Writeoff).count();
                if !has_other && writeoffs >= 2 {
                    out.push(play(Card::Writeoff as u8, 0, 0));
                }
            }
            Card::Survey | Card::Overtime | Card::Subsidy => out.push(play(0, 0, 0)),
        }
    }

    /// Every pair of connected stations with something to carry between them.
    pub fn routes(&self) -> Vec<(u8, u8)> {
        let stations: Vec<u8> = self.stations().collect();
        let mut out = Vec::new();
        for (i, &a) in stations.iter().enumerate() {
            for &b in &stations[i + 1..] {
                if self.route_carries(a, b) && self.rail_path(a, b).is_some() {
                    // The end that loads first goes first.
                    let a_gives = self.cells[a as usize]
                        .site
                        .is_some_and(|s| !s.provides().is_empty());
                    out.push(if a_gives { (a, b) } else { (b, a) });
                }
            }
        }
        out
    }

    pub fn apply(&mut self, mv: Move) {
        self.events.clear();
        match mv {
            Move::EndTurn => self.end_turn(),
            Move::Pick(card) => {
                if let Some(card) = card {
                    self.draw.push(card);
                }
                self.phase = Phase::Remove;
            }
            Move::Remove(card) => {
                if let Some(card) = card {
                    if let Some(i) = self.draw.iter().position(|&c| c == card) {
                        self.draw.swap_remove(i);
                        self.money -= self.cfg.remove_cost;
                    }
                }
                self.begin_stage();
            }
            Move::Play { card, a, b, c } => self.play(card, a, b, c),
        }
    }

    fn play(&mut self, card: Card, a: u8, b: u8, c: u8) {
        let Some(i) = self.hand.iter().position(|&h| h == card) else {
            return;
        };
        self.hand.remove(i);
        let stats = self.cfg.card(card).clone();
        self.energy = self.energy.saturating_sub(stats.cost);
        let mut gone = false;
        match card {
            Card::Path | Card::Mainline | Card::Bridge | Card::Tunnel => {
                if let Some(route) = self.track_route(card, a) {
                    for cell in route {
                        self.cells[cell as usize].track = true;
                    }
                }
            }
            Card::Station => {
                let cell = &mut self.cells[a as usize];
                cell.track = true;
                cell.station = true;
            }
            Card::Loco | Card::Freight | Card::Express => {
                if let Some(path) = self.rail_path(a, b) {
                    self.trains.push(Train {
                        kind: card,
                        speed: stats.speed,
                        capacity: stats.capacity,
                        a,
                        b,
                        path,
                        at: 0,
                        forward: true,
                        cargo: None,
                    });
                    let t = self.trains.len() - 1;
                    self.arrive(t, a);
                }
            }
            Card::Dispatcher => {
                if let Some(path) = self.rail_path(b, c) {
                    let train = &mut self.trains[a as usize];
                    train.a = b;
                    train.b = c;
                    train.path = path;
                    train.at = 0;
                    train.forward = true;
                    let t = a as usize;
                    self.arrive(t, b);
                }
            }
            Card::Survey | Card::Overtime => {
                self.energy += stats.energy;
                self.draw_cards(stats.draw as usize);
            }
            Card::Warehouse => self.cells[a as usize].warehouse = true,
            Card::Subsidy => {
                self.money += self.cfg.subsidy_money;
                self.debt += self.cfg.subsidy_debt;
            }
            Card::Shares => {
                self.money -= self.cfg.shares_price;
                self.shares += 1;
            }
            Card::Timetable => {
                for t in 0..self.trains.len() {
                    let speed = self.trains[t].speed;
                    self.move_train(t, speed);
                }
            }
            Card::Writeoff => {
                let target = Card::ALL[a as usize];
                if let Some(j) = self.hand.iter().position(|&h| h == target) {
                    self.hand.remove(j);
                }
                // The write-off goes with what it wrote off.
                gone = true;
            }
        }
        if !gone {
            self.discard.push(card);
        }
    }
}

impl Terrain {
    fn from_card(card: Card) -> Terrain {
        match card {
            Card::Bridge => Terrain::Water,
            Card::Tunnel => Terrain::Hill,
            _ => Terrain::Plain,
        }
    }
}

#[cfg(test)]
mod tests;
