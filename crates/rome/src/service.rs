//! service: the numbers a city's service walkers live by, read from `content/rome/service.toml`.
//!
//! The rules are Julius' and Augustus' (the open Caesar III re-implementations): fifty
//! ticks to a day, fifteen ticks to a tile, a walker that sets the coverage of every
//! populated house within two tiles of each road tile to 96, a coverage that decays by
//! one a day, a roam length and a spawn-delay ladder per service, and a labour pool that
//! gives a short city its workers by category. The table names the Julius function each
//! number was read from and says how sure we are of it; the house ladder is ours,
//! because Julius reads it from the original game's data file, which is not in its tree.
//!
//! The table is embedded at build time, so two loads hash the same and the tick never
//! reads a file. Values are integers only.

use crate::certainty::Certainty;
use crate::table::{Entry, Table};
use crate::{int, only, LoadError};
use engine_core::Fnv64;

/// How many services the table holds: prefect, temple, market, bathhouse and well.
pub const SERVICES: usize = 5;

/// How many house levels the ladder holds.
pub const LEVELS: usize = 6;

/// How many labour categories Julius counts.
pub const CATEGORIES: usize = 9;

/// The categories in the order Julius' round robin visits them when the pool is short:
/// engineering, water, prefectures, military, food, industry, entertainment, health and
/// education, governance and religion (`DEFAULT_PRIORITY` in `city/labor.c`).
pub const ROUND_ROBIN: [usize; CATEGORIES] = [2, 3, 4, 5, 1, 0, 6, 7, 8];

/// A service building of the city.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    /// The prefecture: its prefect keeps the houses watched.
    Prefect,
    /// The temple: its priest reaches the houses a god looks after.
    Temple,
    /// The market: its market lady reaches the houses that buy there.
    Market,
    /// The bathhouse: its worker reaches the houses that bathe.
    Bathhouse,
    /// The well: nobody walks, the houses next to it have water.
    Well,
}

impl Service {
    /// Every service, in table order.
    pub const ALL: [Self; SERVICES] = [
        Self::Prefect,
        Self::Temple,
        Self::Market,
        Self::Bathhouse,
        Self::Well,
    ];

    /// The table entry this service reads, for example `service.prefect`.
    #[must_use]
    pub const fn entry(self) -> &'static str {
        match self {
            Self::Prefect => "service.prefect",
            Self::Temple => "service.temple",
            Self::Market => "service.market",
            Self::Bathhouse => "service.bathhouse",
            Self::Well => "service.well",
        }
    }

    /// The place of the service in the table's list.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Prefect => 0,
            Self::Temple => 1,
            Self::Market => 2,
            Self::Bathhouse => 3,
            Self::Well => 4,
        }
    }

    /// True when the service sends a walker out. A well does not.
    #[must_use]
    pub const fn walks(self) -> bool {
        !matches!(self, Self::Well)
    }
}

/// One service's rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule<'a> {
    /// The service this rule is.
    pub service: Service,
    /// Tiles on a side of the building.
    pub size: u8,
    /// Workers the building needs to run in full.
    pub laborers: u8,
    /// Ticks the walker roams before it turns for home (a well has none).
    pub roam_ticks: u16,
    /// The labour category the building's workers come from.
    pub category: u8,
    /// Days between walkers at 100, 75, 50, 25 and 1 percent staffing.
    pub delays: [u8; 5],
    /// Tiles a well reaches (zero for a service that walks).
    pub radius: u8,
    /// Where the numbers come from.
    pub source: &'a str,
    /// How sure we are of them.
    pub certainty: Certainty,
}

impl Rule<'_> {
    /// Days to wait between walkers for a building staffed to `percent` of its laborers,
    /// or `None` when it has no workers and so spawns nothing.
    #[must_use]
    pub fn delay(&self, percent: u32) -> Option<u8> {
        match percent {
            100.. => Some(self.delays[0]),
            75..=99 => Some(self.delays[1]),
            50..=74 => Some(self.delays[2]),
            25..=49 => Some(self.delays[3]),
            1..=24 => Some(self.delays[4]),
            0 => None,
        }
    }
}

/// The day's ticks at which each yearly chore of the city runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    /// Ticks in a day.
    pub ticks_per_day: u16,
    /// The tick that decays the count of houses a building covered.
    pub covered: u16,
    /// The tick that lets a populated house take in people.
    pub growth: u16,
    /// The tick that staffs the buildings from the labour pool.
    pub labour: u16,
    /// The tick that marks the houses next to a well.
    pub water: u16,
    /// The tick that spawns walkers.
    pub spawn: u16,
    /// The tick that decays coverage.
    pub decay: u16,
    /// The tick that evolves and devolves the houses.
    pub evolve: u16,
}

/// How a walker walks and how coverage lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Walk {
    /// Ticks a walker takes to cross a tile.
    pub ticks_per_tile: u8,
    /// Tiles around a road tile whose houses a walker covers.
    pub radius: u8,
    /// The coverage a house is set to.
    pub max_coverage: u8,
    /// The coverage a house loses each day.
    pub decay_per_day: u8,
    /// The cap on the count of houses a building covered.
    pub covered_cap: u16,
    /// The most walkers alive in a city.
    pub walker_cap: u8,
}

/// One rung of the house ladder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level<'a> {
    /// The house's name.
    pub name: &'a str,
    /// The people the house holds.
    pub max_people: u16,
    /// Water needed: 0 none, 1 a well.
    pub water: u8,
    /// Temples that must have reached the house.
    pub religion: u8,
    /// Bathhouse coverage needed.
    pub bathhouse: u8,
    /// Market coverage needed.
    pub market: u8,
    /// Where the numbers come from.
    pub source: &'a str,
    /// How sure we are of them.
    pub certainty: Certainty,
}

/// The whole table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceRules<'a> {
    /// When the chores of a day run.
    pub clock: Clock,
    /// How walkers walk and how coverage lasts.
    pub walk: Walk,
    rules: [Rule<'a>; SERVICES],
    levels: [Level<'a>; LEVELS],
    /// Percent of the people who are of working age.
    pub working_age_percent: u8,
    /// Workers a category gets per round robin pass, by Julius category number.
    pub quanta: [u8; CATEGORIES],
    /// People a populated house with road access takes in each day.
    pub people_per_day: u16,
    /// Days a house fails its own needs before it devolves.
    pub devolve_grace: u8,
}

impl ServiceRules<'static> {
    /// The table shipped in `content/rome/service.toml`.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/service.toml");

    /// Read the shipped table.
    ///
    /// # Errors
    ///
    /// Same as [`ServiceRules::parse`]; a test keeps the shipped table valid.
    pub fn shipped() -> Result<Self, LoadError<'static>> {
        Self::parse(Self::SHIPPED)
    }
}

impl<'a> ServiceRules<'a> {
    /// Read one service table.
    ///
    /// # Errors
    ///
    /// [`LoadError::Table`] for a malformed table or a numbered section with no `ref`,
    /// and the typed variants for a missing, unknown, zero or too large value.
    pub fn parse(text: &'a str) -> Result<Self, LoadError<'a>> {
        let table = Table::parse(text)?;
        let clock = read_clock(&table)?;
        let walk = read_walk(&table)?;
        let labour = entry(&table, "labour")?;
        only(
            labour,
            &[
                "working_age_percent",
                "quantum_industry",
                "quantum_food",
                "quantum_engineering",
                "quantum_water",
                "quantum_prefectures",
                "quantum_military",
                "quantum_entertainment",
                "quantum_health",
                "quantum_governance",
                "certainty",
            ],
        )?;
        certainty(labour)?;
        let quanta = [
            small(labour, "quantum_industry")?,
            small(labour, "quantum_food")?,
            small(labour, "quantum_engineering")?,
            small(labour, "quantum_water")?,
            small(labour, "quantum_prefectures")?,
            small(labour, "quantum_military")?,
            small(labour, "quantum_entertainment")?,
            small(labour, "quantum_health")?,
            small(labour, "quantum_governance")?,
        ];
        let growth = entry(&table, "growth")?;
        only(
            growth,
            &["people_per_day", "devolve_grace_days", "certainty"],
        )?;
        certainty(growth)?;
        let mut rules = [read_rule(&table, Service::Prefect)?; SERVICES];
        for service in Service::ALL {
            rules[service.index()] = read_rule(&table, service)?;
        }
        let mut levels = [read_level(&table, 0)?; LEVELS];
        for (at, level) in levels.iter_mut().enumerate() {
            *level = read_level(&table, at)?;
        }
        Ok(Self {
            clock,
            walk,
            rules,
            levels,
            working_age_percent: small(labour, "working_age_percent")?,
            quanta,
            people_per_day: wide(growth, "people_per_day")?,
            devolve_grace: small(growth, "devolve_grace_days")?,
        })
    }

    /// The rule of one service.
    #[must_use]
    pub fn rule(&self, service: Service) -> &Rule<'a> {
        &self.rules[service.index()]
    }

    /// One rung of the house ladder.
    #[must_use]
    pub fn level(&self, level: usize) -> &Level<'a> {
        &self.levels[level.min(LEVELS - 1)]
    }

    /// A digest of the loaded table: two loads of one table agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/service/1");
        for tick in [
            self.clock.ticks_per_day,
            self.clock.covered,
            self.clock.growth,
            self.clock.labour,
            self.clock.water,
            self.clock.spawn,
            self.clock.decay,
            self.clock.evolve,
        ] {
            hasher.write_u16(tick);
        }
        for byte in [
            self.walk.ticks_per_tile,
            self.walk.radius,
            self.walk.max_coverage,
            self.walk.decay_per_day,
            self.walk.walker_cap,
            self.working_age_percent,
            self.devolve_grace,
        ] {
            hasher.write_u8(byte);
        }
        hasher.write_u16(self.walk.covered_cap);
        hasher.write_u16(self.people_per_day);
        hasher.write(&self.quanta);
        for rule in &self.rules {
            hasher.write_u8(rule.size);
            hasher.write_u8(rule.laborers);
            hasher.write_u16(rule.roam_ticks);
            hasher.write_u8(rule.category);
            hasher.write(&rule.delays);
            hasher.write_u8(rule.radius);
        }
        for level in &self.levels {
            hasher.write_u16(level.max_people);
            hasher.write(&[level.water, level.religion, level.bathhouse, level.market]);
        }
        hasher.finish()
    }
}

fn entry<'a, 't>(table: &'t Table<'a>, key: &'static str) -> Result<&'t Entry<'a>, LoadError<'a>> {
    table
        .entry(key)
        .ok_or(LoadError::MissingEntry { entry: key })
}

fn certainty<'a>(entry: &Entry<'a>) -> Result<Certainty, LoadError<'a>> {
    let word = entry
        .text("certainty")
        .ok_or_else(|| LoadError::MissingCertainty { entry: entry.key() })?;
    Certainty::parse(word).ok_or_else(|| LoadError::BadValue {
        entry: entry.key(),
        key: "certainty",
        value: word,
    })
}

/// A required integer that fits `u8`.
fn small<'a, T: TryFrom<u64>>(entry: &Entry<'a>, key: &'static str) -> Result<T, LoadError<'a>> {
    T::try_from(int(entry, key)?).map_err(|_| LoadError::NumberOverflow {
        entry: entry.key(),
        key,
    })
}

/// A required integer that fits `u16`.
fn wide<'a>(entry: &Entry<'a>, key: &'static str) -> Result<u16, LoadError<'a>> {
    small(entry, key)
}

/// A required integer above zero.
fn positive<'a, T: TryFrom<u64> + PartialEq + Default>(
    entry: &Entry<'a>,
    key: &'static str,
) -> Result<T, LoadError<'a>> {
    let value: T = small(entry, key)?;
    if value == T::default() {
        return Err(LoadError::Zero {
            entry: entry.key(),
            key,
        });
    }
    Ok(value)
}

fn read_clock<'a>(table: &Table<'a>) -> Result<Clock, LoadError<'a>> {
    let clock = entry(table, "clock")?;
    only(
        clock,
        &[
            "ticks_per_day",
            "covered_tick",
            "growth_tick",
            "labour_tick",
            "water_tick",
            "spawn_tick",
            "decay_tick",
            "evolve_tick",
            "certainty",
        ],
    )?;
    certainty(clock)?;
    let ticks_per_day: u16 = positive(clock, "ticks_per_day")?;
    let tick = |key: &'static str| -> Result<u16, LoadError<'a>> {
        let value = wide(clock, key)?;
        if value >= ticks_per_day {
            return Err(LoadError::NumberOverflow {
                entry: clock.key(),
                key,
            });
        }
        Ok(value)
    };
    Ok(Clock {
        ticks_per_day,
        covered: tick("covered_tick")?,
        growth: tick("growth_tick")?,
        labour: tick("labour_tick")?,
        water: tick("water_tick")?,
        spawn: tick("spawn_tick")?,
        decay: tick("decay_tick")?,
        evolve: tick("evolve_tick")?,
    })
}

fn read_walk<'a>(table: &Table<'a>) -> Result<Walk, LoadError<'a>> {
    let walk = entry(table, "walk")?;
    only(
        walk,
        &[
            "ticks_per_tile",
            "radius",
            "max_coverage",
            "decay_per_day",
            "covered_cap",
            "certainty",
        ],
    )?;
    certainty(walk)?;
    let walker = entry(table, "walker")?;
    only(walker, &["cap", "certainty"])?;
    certainty(walker)?;
    Ok(Walk {
        ticks_per_tile: positive(walk, "ticks_per_tile")?,
        radius: positive(walk, "radius")?,
        max_coverage: positive(walk, "max_coverage")?,
        decay_per_day: positive(walk, "decay_per_day")?,
        covered_cap: positive(walk, "covered_cap")?,
        walker_cap: positive(walker, "cap")?,
    })
}

fn read_rule<'a>(table: &Table<'a>, service: Service) -> Result<Rule<'a>, LoadError<'a>> {
    let row = entry(table, service.entry())?;
    let walks = service.walks();
    if walks {
        only(
            row,
            &[
                "size",
                "laborers",
                "roam_ticks",
                "category",
                "delay_100",
                "delay_75",
                "delay_50",
                "delay_25",
                "delay_1",
                "certainty",
            ],
        )?;
    } else {
        only(row, &["size", "laborers", "radius", "certainty"])?;
    }
    let category: u8 = if walks { small(row, "category")? } else { 0 };
    if usize::from(category) >= CATEGORIES {
        return Err(LoadError::NumberOverflow {
            entry: row.key(),
            key: "category",
        });
    }
    let delays = if walks {
        [
            small(row, "delay_100")?,
            small(row, "delay_75")?,
            small(row, "delay_50")?,
            small(row, "delay_25")?,
            small(row, "delay_1")?,
        ]
    } else {
        [0; 5]
    };
    Ok(Rule {
        service,
        size: positive(row, "size")?,
        laborers: if walks {
            positive(row, "laborers")?
        } else {
            small(row, "laborers")?
        },
        roam_ticks: if walks {
            positive(row, "roam_ticks")?
        } else {
            0
        },
        category,
        delays,
        radius: if walks { 0 } else { positive(row, "radius")? },
        source: row.source(),
        certainty: certainty(row)?,
    })
}

fn read_level<'a>(table: &Table<'a>, at: usize) -> Result<Level<'a>, LoadError<'a>> {
    const KEYS: [&str; LEVELS] = [
        "house.l0", "house.l1", "house.l2", "house.l3", "house.l4", "house.l5",
    ];
    let row = entry(table, KEYS[at])?;
    only(
        row,
        &[
            "name",
            "max_people",
            "water",
            "religion",
            "bathhouse",
            "market",
            "certainty",
        ],
    )?;
    let name = row.text("name").ok_or_else(|| LoadError::MissingKey {
        entry: row.key(),
        key: "name",
    })?;
    Ok(Level {
        name,
        max_people: positive(row, "max_people")?,
        water: small(row, "water")?,
        religion: small(row, "religion")?,
        bathhouse: small(row, "bathhouse")?,
        market: small(row, "market")?,
        source: row.source(),
        certainty: certainty(row)?,
    })
}
