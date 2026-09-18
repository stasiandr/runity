//! A village that grows on its own — the simulation foundation with no
//! renderer attached.
//!
//! ```text
//! cargo run --release --example settlement
//! ```
//!
//! Nothing here is a game yet. What it demonstrates is the layer a game like
//! that needs underneath it, and that the pieces fit together:
//!
//! * the world runs on its own clock, in whole ticks, with a calendar and
//!   seasons rather than on frames;
//! * every random choice comes from a named, seeded stream, and the terrain
//!   from hash-based noise, so the same seed always grows the same village;
//! * villagers, buildings and land are entities queried by component;
//! * systems talk to each other through events;
//! * and the whole thing saves and loads byte-exactly, which is what makes
//!   the save format, the network format and the test snapshot the same
//!   thing.

use runity::prelude::*;

/// How many people the valley can support before the good ground runs out.
const LAND: f32 = 90.0;

const SAVE_KIND: [u8; 4] = *b"VILL";
const SAVE_VERSION: u32 = 1;

// ----------------------------------------------------------------- components

/// What a villager spends their day doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    /// Brings in food, badly in winter.
    Forager,
    /// Brings in wood, which everything else is built from.
    Woodcutter,
    /// Turns wood into buildings.
    Builder,
}

serializable_enum!(Job {
    0 => Forager,
    1 => Woodcutter,
    2 => Builder,
});

/// Somebody who lives here.
#[derive(Debug, Clone, PartialEq)]
struct Person {
    name: String,
    job: Job,
    /// Rises when there is nothing to eat; past 1.0 they do not survive it.
    hunger: f32,
    days: u32,
}

serializable!(Person {
    name,
    job,
    hunger,
    days
});

/// Where in the valley someone works, and how good that ground is.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Plot {
    position: Vec3,
    fertility: f32,
    forest: f32,
}

serializable!(Plot {
    position,
    fertility,
    forest
});

/// A building, finished or under construction.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Building {
    kind: Structure,
    /// Wood still needed before it is done.
    remaining: f32,
}

serializable!(Building { kind, remaining });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Structure {
    /// Room for four more villagers.
    House,
    /// Keeps food from spoiling over winter.
    Granary,
}

serializable_enum!(Structure {
    0 => House,
    1 => Granary,
});

// --------------------------------------------------------------------- events

/// Somebody was born, or arrived. Carries the entity, because everything
/// about them can be looked up from it.
#[derive(Debug, Clone, Copy)]
struct Born(Entity);

/// Somebody did not make it. Carries the name rather than the entity: by the
/// time anyone reads this, there is nothing left to look up.
#[derive(Debug, Clone)]
struct Died(String);

#[derive(Debug, Clone, Copy)]
struct Finished(Structure);

// ---------------------------------------------------------------- the village

/// Everything the settlement owns collectively.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Stock {
    wood: f32,
    food: f32,
}

serializable!(Stock { wood, food });

struct Village {
    world: World,
    clock: WorldClock,
    stock: Stock,
    seed: u64,
    terrain: Noise,
    forest: Noise,
    rng: Rng,
    names: Rng,
    /// This year's news, gathered from events as the ticks go by.
    year: Chronicle,
}

/// What happened since the last report.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Chronicle {
    born: u32,
    lost: u32,
    houses: u32,
    granaries: u32,
    newest: Option<String>,
    mourned: Option<String>,
}

impl Village {
    /// A fresh valley with three settlers in it.
    fn found(seed: u64) -> Self {
        let mut village = Village {
            world: World::new(),
            // One tick an hour, 360-day years: coarse enough to simulate
            // decades in a moment, fine enough that day and night matter.
            clock: WorldClock::new(4.0).with_calendar(Calendar::new(24, 360)),
            stock: Stock {
                wood: 40.0,
                food: 60.0,
            },
            seed,
            terrain: Noise::named(seed, "fertility"),
            forest: Noise::named(seed, "forest"),
            rng: Rng::named(seed, "village"),
            names: Rng::named(seed, "names"),
            year: Chronicle::default(),
        };
        for _ in 0..3 {
            village.settle(Job::Forager);
        }
        village.settle(Job::Woodcutter);
        village.settle(Job::Builder);
        village.build(Structure::House);
        village
    }

    /// Add a villager on a plot chosen from the terrain.
    fn settle(&mut self, job: Job) -> Entity {
        let position = Vec3::new(
            self.rng.range(-40.0, 40.0),
            0.0,
            self.rng.range(-40.0, 40.0),
        );
        let sample = vec2(position.x * 0.03, position.z * 0.03);
        let plot = Plot {
            position,
            // Fertility and forest come from two independent noise fields, so
            // good farmland and good timber are not the same places.
            fertility: (self.terrain.fbm_2d(sample, Fbm::TERRAIN) * 0.5 + 0.5).clamp(0.05, 1.0),
            forest: (self.forest.ridged_2d(sample, Fbm::TERRAIN.at(0.7)) * 0.5 + 0.5)
                .clamp(0.05, 1.0),
        };

        let name = self.name();
        let person = self.world.spawn();
        self.world.insert(
            person,
            Person {
                name,
                job,
                hunger: 0.0,
                days: 0,
            },
        );
        self.world.insert(person, plot);
        self.world.send(Born(person));
        person
    }

    /// Village names, assembled from syllables so the example needs no data
    /// file — and reproducibly, because the name stream is its own.
    fn name(&mut self) -> String {
        const FIRST: [&str; 8] = ["Mir", "Rad", "Vel", "Stan", "Bor", "Dan", "Lub", "Zor"];
        const LAST: [&str; 6] = ["oslav", "omir", "ana", "ika", "uta", "ena"];
        let first = self.names.pick(&FIRST).copied().unwrap_or("Mir");
        let last = self.names.pick(&LAST).copied().unwrap_or("oslav");
        format!("{first}{last}")
    }

    /// Start a building. It takes wood and a builder's time to finish.
    fn build(&mut self, kind: Structure) -> Entity {
        let remaining = match kind {
            Structure::House => 30.0,
            Structure::Granary => 45.0,
        };
        let site = self.world.spawn();
        self.world.insert(site, Building { kind, remaining });
        site
    }

    fn population(&self) -> usize {
        self.world.count::<Person>()
    }

    /// Four to a house, and nobody moves in before the roof is on.
    fn capacity(&self) -> usize {
        self.world
            .iter::<Building>()
            .filter(|(_, b)| b.kind == Structure::House && b.remaining <= 0.0)
            .count()
            * 4
    }

    fn finished(&self, kind: Structure) -> usize {
        self.world
            .iter::<Building>()
            .filter(|(_, b)| b.kind == kind && b.remaining <= 0.0)
            .count()
    }

    // ------------------------------------------------------------- one tick

    /// One hour of village life.
    fn tick(&mut self) {
        self.world.advance_tick();
        let date = self.clock.date();
        let season = date.season;
        let daylight = !date.is_night();

        // The valley is only so big: every extra mouth works poorer ground
        // than the one before. Without something like this the village grows
        // without limit, which is a simulation of nothing.
        let crowding = LAND / (LAND + self.population() as f32);

        self.work(daylight, season, crowding);
        self.eat();
        self.construct();
        self.grow(date);
        self.read_the_news();
    }

    /// Drain this tick's events into the year's tally.
    ///
    /// This is what events are for: the systems above never had to know that
    /// anyone was counting.
    fn read_the_news(&mut self) {
        let arrivals: Vec<Entity> = self.world.events::<Born>().iter().map(|e| e.0).collect();
        self.year.born += arrivals.len() as u32;
        if let Some(entity) = arrivals.last() {
            self.year.newest = self.world.get::<Person>(*entity).map(|p| p.name.clone());
        }
        if let Some(event) = self.world.events::<Died>().last() {
            self.year.mourned = Some(event.0.clone());
        }
        self.year.lost += self.world.events::<Died>().len() as u32;
        for event in self.world.events::<Finished>() {
            match event.0 {
                Structure::House => self.year.houses += 1,
                Structure::Granary => self.year.granaries += 1,
            }
        }
    }

    /// Foragers and woodcutters bring things in, but only by daylight and
    /// only as well as their plot allows.
    fn work(&mut self, daylight: bool, season: Season, crowding: f32) {
        if !daylight {
            return;
        }
        let harvest = match season {
            Season::Spring => 0.8,
            Season::Summer => 1.2,
            Season::Autumn => 1.0,
            // Winter is the season a settlement has to have prepared for.
            Season::Winter => 0.4,
        };

        let mut wood = 0.0;
        let mut food = 0.0;
        self.world.each2_mut::<Person, Plot>(|_, mut person, plot| {
            match person.job {
                Job::Forager => food += plot.fertility * harvest * crowding * 0.5,
                Job::Woodcutter => wood += plot.forest * crowding * 0.35,
                Job::Builder => {}
            }
            // Reading a field would not mark anything; the day's work does.
            person.hunger = (person.hunger - 0.01).max(0.0);
        });
        self.stock.wood += wood;
        self.stock.food += food;
    }

    /// Everyone eats. When there is nothing, hunger rises, and eventually
    /// somebody does not make it through the winter.
    fn eat(&mut self) {
        let mut starving = Vec::new();
        let stock = &mut self.stock;
        self.world
            .iter_mut::<Person>()
            .for_each(|(entity, mut person)| {
                let needed = 0.02;
                if stock.food >= needed {
                    stock.food -= needed;
                    person.hunger = (person.hunger - 0.02).max(0.0);
                } else {
                    person.hunger += 0.004;
                    if person.hunger > 1.0 {
                        starving.push(entity);
                    }
                }
            });
        for entity in starving {
            let name = self
                .world
                .get::<Person>(entity)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            self.world.despawn(entity);
            self.world.send(Died(name));
        }
    }

    /// Builders turn stockpiled wood into finished buildings.
    fn construct(&mut self) {
        let builders = self
            .world
            .iter::<Person>()
            .filter(|(_, person)| person.job == Job::Builder)
            .count();
        if builders == 0 {
            return;
        }

        let mut effort = builders as f32 * 0.25;
        let mut completed = Vec::new();
        let stock = &mut self.stock;
        for (entity, mut building) in self.world.iter_mut::<Building>() {
            if building.remaining <= 0.0 || effort <= 0.0 {
                continue;
            }
            let spent = effort.min(building.remaining).min(stock.wood);
            if spent <= 0.0 {
                break;
            }
            stock.wood -= spent;
            building.remaining -= spent;
            effort -= spent;
            if building.remaining <= 0.0 {
                completed.push((entity, building.kind));
            }
        }
        for (_, kind) in completed {
            self.world.send(Finished(kind));
        }
    }

    /// Once a day: decide whether the village can feed another mouth, whether
    /// it needs another house, and who should change trade.
    fn grow(&mut self, date: Date) {
        if date.tick_of_day != 0 {
            return;
        }
        for (_, mut person) in self.world.iter_mut::<Person>() {
            person.days += 1;
        }

        self.reassign();
        self.spoil();
        self.winter_toll(date.season);

        let population = self.population();
        let capacity = self.capacity();

        // A full store, a spare bed and a growing season make a new villager.
        let growing = matches!(date.season, Season::Spring | Season::Summer);
        let comfortable = self.stock.food > 60.0 + population as f32 * 6.0;
        if growing && comfortable && population < capacity {
            let job = self.needed_job();
            self.settle(job);
            self.stock.food -= 30.0;
        }

        // Somewhere to put them, built in advance of needing it.
        let unfinished = self
            .world
            .iter::<Building>()
            .filter(|(_, b)| b.remaining > 0.0)
            .count();
        if unfinished == 0 && self.stock.wood > 35.0 && population + 2 >= capacity {
            self.build(Structure::House);
        }
        // And somewhere to keep the winter's food once there are enough
        // people to fill a granary.
        // Winter is coming whether anyone prepared for it or not, so the
        // store goes up before the village is big enough to need it.
        if unfinished == 0
            && population >= 6
            && self.finished(Structure::Granary) < 1 + population / 12
        {
            self.build(Structure::Granary);
        }
    }

    /// Winter takes some of them, and takes more when the village is hungry
    /// or crowded.
    ///
    /// Every roll comes from the village's own named stream, so a bad winter
    /// is part of this world's history rather than of this run's luck.
    fn winter_toll(&mut self, season: Season) {
        if season != Season::Winter {
            return;
        }
        let crowding = 1.0 + self.population() as f32 / LAND;
        let at_risk: Vec<(Entity, f32)> = self
            .world
            .iter::<Person>()
            .map(|(entity, person)| (entity, person.hunger))
            .collect();
        let mut lost = Vec::new();
        for (entity, hunger) in at_risk {
            if self.rng.chance(0.0008 * crowding * (1.0 + hunger * 6.0)) {
                lost.push(entity);
            }
        }
        for entity in lost {
            let name = self
                .world
                .get::<Person>(entity)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            self.world.despawn(entity);
            self.world.send(Died(name));
        }
    }

    /// Villagers change trade to whatever the village is short of.
    ///
    /// Without this the settlement deadlocks in its first year: the house
    /// needs a builder, and nobody becomes a builder until there is a house
    /// to be born into.
    fn reassign(&mut self) {
        let unfinished = self
            .world
            .iter::<Building>()
            .filter(|(_, b)| b.remaining > 0.0)
            .count();
        let builders = self.count_job(Job::Builder);
        let woodcutters = self.count_job(Job::Woodcutter);

        if self.stock.food < 12.0 {
            // Nothing else matters if the village is hungry.
            self.retrain(Job::Builder, Job::Forager);
            self.retrain(Job::Woodcutter, Job::Forager);
            return;
        }
        if unfinished > 0 && self.stock.wood > 5.0 && builders == 0 {
            // A woodcutter first: the village needs the timber less than it
            // needs somebody to nail it together.
            let _ = self.retrain(Job::Woodcutter, Job::Builder)
                || self.retrain(Job::Forager, Job::Builder);
        }
        if unfinished == 0 && builders > 0 {
            self.retrain(Job::Builder, Job::Forager);
        }
        if woodcutters == 0 && self.stock.wood < 40.0 {
            self.retrain(Job::Forager, Job::Woodcutter);
        }
    }

    /// Move one villager from one trade to another, if there is one to move.
    fn retrain(&mut self, from: Job, to: Job) -> bool {
        for (_, mut person) in self.world.iter_mut::<Person>() {
            if person.job == from {
                person.job = to;
                return true;
            }
        }
        false
    }

    fn count_job(&self, job: Job) -> usize {
        self.world
            .iter::<Person>()
            .filter(|(_, p)| p.job == job)
            .count()
    }

    /// Food rots and timber warps. What the village can keep is what it has
    /// built somewhere to keep it — which is the entire point of a granary.
    fn spoil(&mut self) {
        let food_capacity = 60.0
            + self.finished(Structure::House) as f32 * 30.0
            + self.finished(Structure::Granary) as f32 * 500.0;
        let wood_capacity = 120.0 + self.finished(Structure::House) as f32 * 60.0;
        self.stock.food = self.stock.food.min(food_capacity);
        self.stock.wood = self.stock.wood.min(wood_capacity);
    }

    /// The trade the village is shortest of right now.
    fn needed_job(&self) -> Job {
        let builders = self
            .world
            .iter::<Person>()
            .filter(|(_, p)| p.job == Job::Builder)
            .count();
        let woodcutters = self
            .world
            .iter::<Person>()
            .filter(|(_, p)| p.job == Job::Woodcutter)
            .count();
        if self.stock.food < 30.0 {
            Job::Forager
        } else if builders * 4 < self.population() {
            Job::Builder
        } else if woodcutters * 3 < self.population() {
            Job::Woodcutter
        } else {
            Job::Forager
        }
    }

    // ------------------------------------------------------------- driving

    /// Run the clock forward by `days` of world time, one tick at a time.
    fn live(&mut self, days: u64) {
        let ticks = days * self.clock.calendar.ticks_per_day;
        let step = self.clock.seconds_per_tick() as f32;
        for _ in 0..ticks {
            // Feed the clock exactly one tick's worth of real time; the world
            // decides for itself how many ticks that is worth.
            self.clock.advance(step);
            while self.clock.next_tick().is_some() {
                self.tick();
            }
        }
    }

    fn report(&self) -> String {
        let date = self.clock.date();
        format!(
            "year {:>2} {:<6}  people {:>3}  houses {:>2}  granaries {}  wood {:>6.1}  food {:>6.1}",
            date.year + 1,
            date.season.name(),
            self.population(),
            self.finished(Structure::House),
            self.finished(Structure::Granary),
            self.stock.wood,
            self.stock.food,
        )
    }

    // -------------------------------------------------------- saving the world

    fn save(&self) -> Vec<u8> {
        Archive::write(SAVE_KIND, SAVE_VERSION, |writer| {
            writer
                .write(&self.seed)
                .write(&self.clock)
                .write(&self.stock)
                // The generators go in the save too: resuming from the seed
                // alone would replay rolls this village has already lived.
                .write(&self.rng)
                .write(&self.names);
            self.world.save_entities(writer);
            self.world.save_components::<Person>(writer);
            self.world.save_components::<Plot>(writer);
            self.world.save_components::<Building>(writer);
        })
    }

    fn load(bytes: &[u8]) -> std::io::Result<Village> {
        let (_, mut reader) = Archive::open(bytes, SAVE_KIND, SAVE_VERSION)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let read = |reader: &mut Reader| -> runity::serialize::Result<Village> {
            let seed: u64 = reader.read()?;
            let clock: WorldClock = reader.read()?;
            let stock: Stock = reader.read()?;
            let rng: Rng = reader.read()?;
            let names: Rng = reader.read()?;
            let mut world = World::new();
            world.load_entities(reader)?;
            world.load_components::<Person>(reader)?;
            world.load_components::<Plot>(reader)?;
            world.load_components::<Building>(reader)?;
            Ok(Village {
                world,
                clock,
                stock,
                seed,
                // The noise fields need no saving at all: they are pure
                // functions of the seed, which is why terrain can be thrown
                // away and regenerated whenever it is convenient.
                terrain: Noise::named(seed, "fertility"),
                forest: Noise::named(seed, "forest"),
                rng,
                names,
                year: Chronicle::default(),
            })
        };
        read(&mut reader).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

fn main() -> std::io::Result<()> {
    let seed = std::env::var("RUNITY_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_260_918);
    let years: u64 = std::env::var("RUNITY_YEARS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12);

    println!("A valley, seed {seed}. Nobody is in charge.\n");
    let mut village = Village::found(seed);
    println!("  {}", village.report());

    for _ in 0..years {
        village.live(360);
        println!("  {}", village.report());
        let news = std::mem::take(&mut village.year);
        if news != Chronicle::default() {
            let newest = news.newest.as_deref().unwrap_or("nobody");
            let mourned = news.mourned.as_deref().unwrap_or("nobody");
            println!(
                "     {} born ({newest} last), {} lost ({mourned} last), {} house(s), {} granary(ies)",
                news.born, news.lost, news.houses, news.granaries
            );
        }
    }

    // --- the world keeps its shape across a save -------------------------
    let bytes = village.save();
    println!("\nsaved {} bytes", bytes.len());

    let mut loaded = Village::load(&bytes)?;
    println!("  loaded: {}", loaded.report());
    assert_eq!(
        loaded.save(),
        bytes,
        "a save must survive its own round trip"
    );

    // And carries on identically from there.
    village.live(360);
    loaded.live(360);
    assert_eq!(
        village.report(),
        loaded.report(),
        "a loaded world must live the same year"
    );
    println!("  a year later, both agree: {}", loaded.report());

    // --- and the same seed always grows the same village ------------------
    let mut twin = Village::found(seed);
    twin.live(360 * years);
    let mut original = Village::found(seed);
    original.live(360 * years);
    assert_eq!(twin.report(), original.report());
    println!("\nsame seed, same history: {}", twin.report());

    let mut other = Village::found(seed + 1);
    other.live(360 * years);
    println!("another valley:            {}", other.report());
    Ok(())
}
