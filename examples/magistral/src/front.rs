//! What the player sees and presses, without the engine: the run, what is
//! picked so far, and the model the screen (`ui/game.screen.html`) shows.
//! The window turns clicks into [`Press`]es; the tests press directly.

use std::collections::VecDeque;
use std::sync::Arc;

use magistral_rules::ai::{Player, Search};
use magistral_rules::{Card, Cargo, Config, Event, Move, Phase, Site, State, Terrain, CELLS, H, W};
use serde::Serialize;

/// How hard the hint thinks: iterations of the search per move.
pub const HINT_SEARCH: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// A card in the hand, by its place.
    Card(usize),
    Cell(u8),
    /// A train in the list on the right.
    Train(usize),
    EndTurn,
    /// A card offered at the market, by its place; `None` takes none.
    Offer(Option<usize>),
    /// A kind of card in the deck at the market, by its row; `None` keeps all.
    Remove(Option<usize>),
    /// Ask the search what it would do; let it do it.
    Hint,
    Auto,
    Cancel,
    NewRun,
}

pub struct Front {
    pub cfg: Arc<Config>,
    pub state: State,
    pub seed: u64,
    /// The card being played, and what has been clicked for it so far.
    selected: Option<Card>,
    picked: Vec<u8>,
    log: VecDeque<String>,
    hint: String,
}

#[derive(Serialize, Default)]
pub struct View {
    pub top: Top,
    pub board: Vec<CellView>,
    pub hand: Vec<CardView>,
    pub contracts: Vec<Line>,
    pub trains: Vec<Line>,
    pub prompt: String,
    pub hint: String,
    pub log: Vec<String>,
    pub market: Market,
    pub over: bool,
    pub result: String,
}

#[derive(Serialize, Default)]
pub struct Top {
    pub stage: String,
    pub turn: String,
    pub money: String,
    pub due: String,
    pub energy: String,
    pub deck: String,
}

#[derive(Serialize, Default)]
pub struct CellView {
    pub color: &'static str,
    pub label: String,
    pub track: bool,
    /// Track goes on to the cell above, below, right, left.
    pub n: bool,
    pub s: bool,
    pub e: bool,
    pub w: bool,
    pub station: bool,
    pub mark: bool,
    pub train: bool,
}

#[derive(Serialize, Default)]
pub struct CardView {
    pub name: String,
    pub text: String,
    pub cost: u8,
    pub selected: bool,
    pub playable: bool,
}

#[derive(Serialize, Default)]
pub struct Line {
    pub text: String,
    pub on: bool,
}

#[derive(Serialize, Default)]
pub struct Market {
    pub pick: bool,
    pub remove: bool,
    pub offers: Vec<CardView>,
    pub deck: Vec<Line>,
    pub title: String,
}

pub fn card_name(card: Card) -> &'static str {
    match card {
        Card::Path => "Путь",
        Card::Mainline => "Магистраль",
        Card::Bridge => "Мост",
        Card::Tunnel => "Тоннель",
        Card::Station => "Станция",
        Card::Loco => "Паровоз",
        Card::Freight => "Товарный",
        Card::Express => "Скорый",
        Card::Survey => "Изыскания",
        Card::Overtime => "Сверхурочные",
        Card::Dispatcher => "Диспетчер",
        Card::Warehouse => "Склад",
        Card::Subsidy => "Субсидия",
        Card::Shares => "Акции",
        Card::Timetable => "Расписание",
        Card::Writeoff => "Списание",
    }
}

pub fn card_text(card: Card, cfg: &Config) -> String {
    let s = cfg.card(card);
    match card {
        Card::Path | Card::Mainline => {
            format!("{} кл. пути по равнине к выбранному месту", s.cells)
        }
        Card::Bridge => format!("{} кл. пути, можно через реку", s.cells),
        Card::Tunnel => format!("{} кл. пути, можно сквозь горы", s.cells),
        Card::Station => "Станция у места рядом с путём".into(),
        Card::Loco | Card::Freight | Card::Express => {
            format!("Поезд: скорость {}, везёт {}", s.speed, s.capacity)
        }
        Card::Survey => format!("Взять {} карты", s.draw),
        Card::Overtime => format!("+{} бригада, взять {}", s.energy, s.draw),
        Card::Dispatcher => "Дать поезду новый маршрут".into(),
        Card::Warehouse => format!("Станция копит до {} груза", cfg.warehouse_cap),
        Card::Subsidy => format!(
            "+{} сейчас, к платежу +{}",
            cfg.subsidy_money, cfg.subsidy_debt
        ),
        Card::Shares => format!(
            "Вложить {}: +{} каждый ход",
            cfg.shares_price, cfg.shares_income
        ),
        Card::Timetable => "Все поезда делают лишний перегон".into(),
        Card::Writeoff => "Убрать карту из руки навсегда".into(),
    }
}

fn cargo_name(cargo: Cargo) -> &'static str {
    match cargo {
        Cargo::Coal => "уголь",
        Cargo::Ore => "руда",
        Cargo::Grain => "зерно",
        Cargo::Steel => "сталь",
    }
}

fn site_letter(site: Site) -> &'static str {
    match site {
        Site::City => "Город",
        Site::Coal => "Уголь",
        Site::Ore => "Руда",
        Site::Farm => "Ферма",
        Site::Factory => "Завод",
    }
}

fn cell_name(state: &State, cell: u8) -> String {
    let (x, y) = magistral_rules::xy(cell);
    let site = state.cells[cell as usize]
        .site
        .map_or("клетка", site_letter);
    format!("{site} {}{}", (b'A' + x as u8) as char, y + 1)
}

impl Front {
    pub fn new(cfg: Arc<Config>, seed: u64) -> Front {
        let state = State::new(cfg.clone(), seed);
        let mut front = Front {
            cfg,
            state,
            seed,
            selected: None,
            picked: Vec::new(),
            log: VecDeque::new(),
            hint: String::new(),
        };
        front.say("Новая дорога. Город с вокзалом слева — тяните путь к углю и ферме.".into());
        front
    }

    fn say(&mut self, line: String) {
        self.log.push_front(line);
        self.log.truncate(7);
    }

    /// Do one move and tell the log what came of it.
    pub fn apply(&mut self, mv: Move) {
        self.state.apply(mv);
        self.selected = None;
        self.picked.clear();
        self.hint.clear();
        let events = std::mem::take(&mut self.state.events);
        let mut delivered = 0;
        for e in events {
            match e {
                Event::Delivered { coins, .. } => delivered += coins,
                Event::ContractDone { reward } => self.say(format!("Контракт выполнен: +{reward}")),
                Event::ContractLost => self.say("Контракт сорван".into()),
                Event::Paid { coins } => self.say(format!("Платёж банку: −{coins}")),
                Event::Bankrupt { owed } => self.say(format!("Банкрот: надо было {owed}")),
                Event::Won => self.say("Кредит выплачен!".into()),
            }
        }
        if delivered > 0 {
            self.say(format!("Поезда привезли +{delivered}"));
        }
    }

    /// The moves of the selected card that agree with what is picked so far.
    fn candidates(&self) -> Vec<Move> {
        let Some(card) = self.selected else {
            return Vec::new();
        };
        self.state
            .moves()
            .into_iter()
            .filter(|mv| match *mv {
                Move::Play {
                    card: c,
                    a,
                    b,
                    c: third,
                } if c == card => match card {
                    Card::Dispatcher => match self.picked.as_slice() {
                        [] => true,
                        [t] => a == *t,
                        [t, s] => a == *t && (b == *s || third == *s),
                        [t, s, u] => a == *t && ((b, third) == (*s, *u) || (third, b) == (*s, *u)),
                        _ => false,
                    },
                    c if c.is_train() => match self.picked.as_slice() {
                        [] => true,
                        [s] => a == *s || b == *s,
                        [s, t] => (a, b) == (*s, *t) || (b, a) == (*s, *t),
                        _ => false,
                    },
                    _ => true,
                },
                _ => false,
            })
            .collect()
    }

    /// The cells that may be clicked next for the selected card.
    fn targets(&self) -> Vec<u8> {
        let Some(card) = self.selected else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for mv in self.candidates() {
            let Move::Play { a, b, c, .. } = mv else {
                continue;
            };
            match card {
                Card::Dispatcher => match self.picked.len() {
                    1 => out.extend([b, c]),
                    2 => out.extend([b, c].into_iter().filter(|x| *x != self.picked[1])),
                    _ => {}
                },
                c if c.is_train() => match self.picked.len() {
                    0 => out.extend([a, b]),
                    _ => out.extend([a, b].into_iter().filter(|x| *x != self.picked[0])),
                },
                Card::Path
                | Card::Mainline
                | Card::Bridge
                | Card::Tunnel
                | Card::Station
                | Card::Warehouse => out.push(a),
                _ => {}
            }
        }
        out.sort();
        out.dedup();
        out
    }

    pub fn press(&mut self, press: Press) {
        match (&self.state.phase, press) {
            (_, Press::NewRun) => {
                let seed = self
                    .seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                *self = Front::new(self.cfg.clone(), seed);
            }
            (Phase::Over, _) => {}
            (_, Press::Hint) => {
                let mv = Search::new(HINT_SEARCH, self.seed ^ self.state.turn as u64)
                    .choose(&self.state);
                self.hint = format!("ИИ советует: {}", self.describe(mv));
            }
            (_, Press::Auto) => {
                let mv = Search::new(HINT_SEARCH, self.seed ^ self.state.turn as u64)
                    .choose(&self.state);
                self.say(format!("ИИ: {}", self.describe(mv)));
                self.apply(mv);
            }
            (Phase::Pick(offers), Press::Offer(i)) => {
                let card = i.and_then(|i| offers.get(i).copied());
                self.apply(Move::Pick(card));
            }
            (Phase::Remove, Press::Remove(row)) => {
                let card = row.and_then(|r| self.deck_kinds().get(r).map(|k| k.0));
                if card.is_none() || self.state.money >= self.cfg.remove_cost {
                    self.apply(Move::Remove(card));
                }
            }
            (Phase::Play, Press::EndTurn) => self.apply(Move::EndTurn),
            (Phase::Play, Press::Cancel) => {
                self.selected = None;
                self.picked.clear();
            }
            (Phase::Play, Press::Card(i)) => self.press_card(i),
            (Phase::Play, Press::Cell(cell)) => self.press_cell(cell),
            (Phase::Play, Press::Train(t)) => {
                if self.selected == Some(Card::Dispatcher) {
                    let t = t as u8;
                    if self
                        .candidates()
                        .iter()
                        .any(|mv| matches!(mv, Move::Play { a, .. } if *a == t))
                    {
                        self.picked = vec![t];
                    }
                }
            }
            _ => {}
        }
    }

    fn press_card(&mut self, i: usize) {
        let Some(&card) = self.state.hand.get(i) else {
            return;
        };
        if self.selected == Some(Card::Writeoff) && card != Card::Writeoff {
            self.apply(Move::Play {
                card: Card::Writeoff,
                a: card as u8,
                b: 0,
                c: 0,
            });
            return;
        }
        if self.selected == Some(card) {
            self.selected = None;
            self.picked.clear();
            return;
        }
        self.selected = Some(card);
        self.picked.clear();
        let moves = self.candidates();
        if moves.is_empty() {
            self.selected = None;
            self.say(format!("«{}» сейчас некуда сыграть", card_name(card)));
            return;
        }
        if matches!(
            card,
            Card::Survey | Card::Overtime | Card::Subsidy | Card::Shares | Card::Timetable
        ) {
            self.apply(moves[0]);
        }
    }

    fn press_cell(&mut self, cell: u8) {
        let Some(card) = self.selected else { return };
        if !self.targets().contains(&cell) {
            return;
        }
        match card {
            Card::Dispatcher | Card::Loco | Card::Freight | Card::Express => {
                self.picked.push(cell);
                let need = if card == Card::Dispatcher { 3 } else { 2 };
                if self.picked.len() == need {
                    let p = &self.picked;
                    let found = self.candidates().into_iter().find(|mv| match *mv {
                        Move::Play { a, b, c, .. } if card == Card::Dispatcher => {
                            a == p[0] && ((b, c) == (p[1], p[2]) || (c, b) == (p[1], p[2]))
                        }
                        Move::Play { a, b, .. } => (a, b) == (p[0], p[1]) || (b, a) == (p[0], p[1]),
                        _ => false,
                    });
                    match found {
                        Some(mv) => self.apply(mv),
                        None => self.picked.truncate(need - 1),
                    }
                }
            }
            _ => {
                if let Some(mv) = self
                    .candidates()
                    .into_iter()
                    .find(|mv| matches!(mv, Move::Play { a, .. } if *a == cell))
                {
                    self.apply(mv);
                }
            }
        }
    }

    fn deck_kinds(&self) -> Vec<(Card, usize)> {
        let mut out: Vec<(Card, usize)> = Vec::new();
        let mut all = self.state.draw.clone();
        all.sort();
        for c in all {
            match out.last_mut() {
                Some((k, n)) if *k == c => *n += 1,
                _ => out.push((c, 1)),
            }
        }
        out
    }

    pub fn describe(&self, mv: Move) -> String {
        match mv {
            Move::EndTurn => "закончить ход".into(),
            Move::Pick(None) => "ничего не брать".into(),
            Move::Pick(Some(c)) => format!("взять «{}»", card_name(c)),
            Move::Remove(None) => "ничего не выкидывать".into(),
            Move::Remove(Some(c)) => format!("выкинуть «{}»", card_name(c)),
            Move::Play { card, a, b, c } => {
                let name = card_name(card);
                match card {
                    Card::Path | Card::Mainline | Card::Bridge | Card::Tunnel => {
                        format!("«{name}» к месту {}", cell_name(&self.state, a))
                    }
                    Card::Station | Card::Warehouse => {
                        format!("«{name}»: {}", cell_name(&self.state, a))
                    }
                    Card::Loco | Card::Freight | Card::Express => {
                        format!(
                            "«{name}»: {} — {}",
                            cell_name(&self.state, a),
                            cell_name(&self.state, b)
                        )
                    }
                    Card::Dispatcher => format!(
                        "«{name}»: поезд {} на {} — {}",
                        a + 1,
                        cell_name(&self.state, b),
                        cell_name(&self.state, c)
                    ),
                    Card::Writeoff => {
                        format!("«{name}»: убрать «{}»", card_name(Card::ALL[a as usize]))
                    }
                    _ => format!("«{name}»"),
                }
            }
        }
    }

    pub fn view(&self) -> View {
        let s = &self.state;
        let targets = self.targets();
        let mut board = Vec::with_capacity(CELLS);
        for i in 0..CELLS {
            let cell = &s.cells[i];
            let color = match (cell.site, cell.terrain) {
                (Some(Site::City), _) => "#e8d9b0",
                (Some(Site::Factory), _) => "#c9b7c9",
                (Some(_), _) => "#d8c9a0",
                (None, Terrain::Water) => "#5b8fd1",
                (None, Terrain::Hill) => "#9c8466",
                (None, Terrain::Plain) => "#b9cc8e",
            };
            let label = match cell.site {
                Some(site) => {
                    let stock: u8 = cell.stock.iter().sum();
                    if cell.station && stock > 0 {
                        format!("{} {}", site_letter(site), stock)
                    } else {
                        site_letter(site).to_string()
                    }
                }
                None => String::new(),
            };
            let (x, y) = magistral_rules::xy(i as u8);
            let joins = |nx: usize, ny: usize| cell.track && s.cells[ny * W + nx].track;
            board.push(CellView {
                color,
                label,
                track: cell.track,
                n: y > 0 && joins(x, y - 1),
                s: y + 1 < H && joins(x, y + 1),
                e: x + 1 < W && joins(x + 1, y),
                w: x > 0 && joins(x - 1, y),
                station: cell.station,
                mark: targets.contains(&(i as u8)) || self.picked.contains(&(i as u8)),
                train: s.trains.iter().any(|t| t.cell() as usize == i),
            });
        }
        let hand = s
            .hand
            .iter()
            .map(|&card| CardView {
                name: card_name(card).into(),
                text: card_text(card, &self.cfg),
                cost: self.cfg.card(card).cost,
                selected: self.selected == Some(card),
                playable: self.cfg.card(card).cost <= s.energy && s.phase == Phase::Play,
            })
            .collect();
        let contracts = s
            .contracts
            .iter()
            .enumerate()
            .map(|(i, c)| Line {
                text: format!(
                    "{}{} {} в {} — {} за {} х.{}",
                    if i == 0 {
                        "Сейчас: "
                    } else {
                        "Потом: "
                    },
                    c.amount,
                    cargo_name(c.cargo),
                    cell_name(s, c.city),
                    c.reward,
                    c.turns,
                    if i == 0 && c.done > 0 {
                        format!(" (есть {})", c.done)
                    } else {
                        String::new()
                    }
                ),
                on: i == 0,
            })
            .collect();
        let trains = s
            .trains
            .iter()
            .enumerate()
            .map(|(i, t)| Line {
                text: format!(
                    "{}. {}: {} — {}{}",
                    i + 1,
                    card_name(t.kind),
                    cell_name(s, t.a),
                    cell_name(s, t.b),
                    t.cargo.map_or(String::new(), |(c, n)| format!(
                        ", везёт {n} {}",
                        cargo_name(c)
                    ))
                ),
                on: self.picked.first() == Some(&(i as u8))
                    && self.selected == Some(Card::Dispatcher),
            })
            .collect();
        let prompt = match (&s.phase, self.selected) {
            (Phase::Play, None) => "Выберите карту в руке или закончите ход.".to_string(),
            (Phase::Play, Some(Card::Writeoff)) => "Нажмите карту, которую убрать навсегда.".into(),
            (Phase::Play, Some(Card::Dispatcher)) if self.picked.is_empty() => {
                "Выберите поезд в списке справа.".into()
            }
            (Phase::Play, Some(c)) if c.is_train() || c == Card::Dispatcher => {
                "Нажмите две станции маршрута (подсвечены).".into()
            }
            (Phase::Play, Some(c)) => format!("«{}»: нажмите подсвеченную клетку.", card_name(c)),
            (Phase::Pick(_), _) => "Биржа: возьмите одну карту или ни одной.".into(),
            (Phase::Remove, _) => format!(
                "Можно выкинуть одну карту из колоды за {}.",
                self.cfg.remove_cost
            ),
            (Phase::Over, _) => String::new(),
        };
        let market = match &s.phase {
            Phase::Pick(offers) => Market {
                pick: true,
                title: format!("Этап {} пройден", s.stage),
                offers: offers
                    .iter()
                    .map(|&c| CardView {
                        name: card_name(c).into(),
                        text: card_text(c, &self.cfg),
                        cost: self.cfg.card(c).cost,
                        selected: false,
                        playable: true,
                    })
                    .collect(),
                ..Default::default()
            },
            Phase::Remove => Market {
                remove: true,
                title: format!(
                    "Выкинуть карту за {} (у вас {})",
                    self.cfg.remove_cost, s.money
                ),
                deck: self
                    .deck_kinds()
                    .into_iter()
                    .map(|(c, n)| Line {
                        text: format!("{} ×{n}", card_name(c)),
                        on: s.money >= self.cfg.remove_cost,
                    })
                    .collect(),
                ..Default::default()
            },
            _ => Market::default(),
        };
        View {
            top: Top {
                stage: format!(
                    "Этап {} из {}",
                    (s.stage as usize + 1).min(s.stages()),
                    s.stages()
                ),
                turn: format!(
                    "Ход {} из {}",
                    (s.turn + 1).min(self.cfg.turns_per_stage),
                    self.cfg.turns_per_stage
                ),
                money: format!("Касса {}", s.money),
                due: format!("Платёж {}", s.payment_due()),
                energy: format!("Бригады {}", s.energy),
                deck: format!("Колода {} · сброс {}", s.draw.len(), s.discard.len()),
            },
            board,
            hand,
            contracts,
            trains,
            prompt,
            hint: self.hint.clone(),
            log: self.log.iter().cloned().collect(),
            market,
            over: s.over(),
            result: if s.won {
                format!("Кредит выплачен! Заработано {}", s.earned)
            } else {
                format!("Банкрот на этапе {}. Заработано {}", s.stage + 1, s.earned)
            },
        }
    }
}
