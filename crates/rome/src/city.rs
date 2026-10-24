//! city: service walkers, house coverage, house evolution and the labour pool.
//!
//! The core mechanic of Caesar III, read from its open re-implementations Julius and
//! Augustus. A service building (prefecture, temple, market, bathhouse) with workers
//! spawns one walker. The walker leaves by road and roams a bounded route: it keeps its
//! way along a road, turns by a pseudo-random pick at a junction, turns round at a dead
//! end, and may not cross a roadblock (Augustus). At each tile it finishes it sets the
//! coverage of every populated house within two tiles to the maximum. When its roam
//! length is spent it walks home by the shortest road, and a walker bound for a
//! destination passes a roadblock a roaming one may not. Coverage decays by one a day.
//! A house that meets the needs of the next level evolves; one that fails the needs of
//! its own level devolves after two days of grace. The labour pool is the whole city's:
//! the working-age people of every house staff the buildings by category, and a
//! building with no workers sends no walker.
//!
//! The day is fifty ticks, and the chores of the day run on their own ticks as in
//! Julius (`game/tick.c`). Everything is integer. All storage is sized when the city is
//! made (grid, building list, walker array, one distance field and its queue), so
//! [`City::tick`] never allocates, and the walker count is capped by the table.

use crate::service::{Service, ServiceRules, CATEGORIES, LEVELS, ROUND_ROBIN};
use engine_core::Fnv64;

/// The longest side of a city, in tiles.
pub const MAX_SIDE: u8 = 64;

/// The most buildings (houses and services) a city holds.
pub const MAX_BUILDINGS: usize = 512;

/// The size of the walker array; the table's cap may be lower, never higher.
pub const MAX_WALKERS: usize = 64;

/// The four ways a walker can go: north, east, south, west.
const STEP: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// No direction: the walker has not moved yet.
const NONE: u8 = 4;

/// A distance no tile has: not reachable.
const FAR: u16 = u16::MAX;

/// What a tile of the grid holds as ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tile {
    /// Nothing: no walker steps here.
    Empty,
    /// A road.
    Road,
    /// A roadblock on a road: roaming walkers stop, walkers with a destination pass.
    Roadblock,
}

/// What a building is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A house: people live here and services reach it.
    House,
    /// A service building.
    Service(Service),
}

impl Kind {
    const fn code(self) -> u8 {
        match self {
            Self::House => 0,
            Self::Service(service) => 1 + service_code(service),
        }
    }
}

const fn service_code(service: Service) -> u8 {
    match service {
        Service::Prefect => 0,
        Service::Temple => 1,
        Service::Market => 2,
        Service::Bathhouse => 3,
        Service::Well => 4,
    }
}

/// One building of the city.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Building {
    used: bool,
    kind: Kind,
    x: u8,
    y: u8,
    size: u8,
    workers: u8,
    spawn_delay: u8,
    walker: u8,
    roam_dir: u8,
    houses_covered: u16,
    level: u8,
    people: u16,
    coverage: [u8; 4],
    well: bool,
    devolve: u8,
}

impl Building {
    const EMPTY: Self = Self {
        used: false,
        kind: Kind::House,
        x: 0,
        y: 0,
        size: 1,
        workers: 0,
        spawn_delay: 0,
        walker: 0,
        roam_dir: 0,
        houses_covered: 0,
        level: 0,
        people: 0,
        coverage: [0; 4],
        well: false,
        devolve: 0,
    };

    /// What the building is.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// Workers the labour pool gave the building.
    #[must_use]
    pub const fn workers(&self) -> u8 {
        self.workers
    }

    /// Houses its walkers reached lately (decays by one a day, capped).
    #[must_use]
    pub const fn houses_covered(&self) -> u16 {
        self.houses_covered
    }

    /// A house's level on the ladder.
    #[must_use]
    pub const fn level(&self) -> u8 {
        self.level
    }

    /// A house's people.
    #[must_use]
    pub const fn people(&self) -> u16 {
        self.people
    }

    /// A house's coverage by a walking service, 0 to the maximum.
    #[must_use]
    pub const fn coverage(&self, service: Service) -> u8 {
        let at = service_code(service) as usize;
        if at < 4 {
            self.coverage[at]
        } else {
            0
        }
    }

    /// True when a well is within reach of the house.
    #[must_use]
    pub const fn has_well(&self) -> bool {
        self.well
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Roam,
    Home,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Walker {
    alive: bool,
    service: Service,
    building: u16,
    x: u8,
    y: u8,
    dir: u8,
    moved: bool,
    progress: u8,
    roam: u16,
    phase: Phase,
    turn: u8,
    counter: u16,
}

impl Walker {
    const DEAD: Self = Self {
        alive: false,
        service: Service::Prefect,
        building: 0,
        x: 0,
        y: 0,
        dir: 0,
        moved: false,
        progress: 0,
        roam: 0,
        phase: Phase::Roam,
        turn: 1,
        counter: 0,
    };
}

/// The city's labour pool after the last staffing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Labour {
    /// Working-age people in the whole city.
    pub available: u32,
    /// Workers the buildings ask for.
    pub needed: u32,
    /// Workers at work.
    pub employed: u32,
    /// Workers with no work.
    pub unemployed: u32,
}

/// Counts of what the walkers did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Walkers sent out.
    pub spawned: u32,
    /// Walkers that walked home and went in.
    pub home: u32,
    /// Walkers that found no way home and were lost.
    pub lost: u32,
    /// Spawns refused because the walker cap was reached.
    pub capped: u32,
}

/// A city: its ground, its buildings, its walkers and its labour pool.
#[derive(Clone, Debug)]
pub struct City {
    rules: ServiceRules<'static>,
    width: u8,
    height: u8,
    seed: u64,
    tiles: Vec<Tile>,
    pass: Vec<u8>,
    owner: Vec<u16>,
    tile_rand: Vec<u8>,
    buildings: Vec<Building>,
    walkers: [Walker; MAX_WALKERS],
    dist: Vec<u16>,
    queue: Vec<u16>,
    tick_of_day: u16,
    day: u32,
    labour: Labour,
    stats: Stats,
}

impl City {
    /// A city of `width` by `height` tiles on the shipped table, or `None` when a side
    /// is zero or above [`MAX_SIDE`].
    #[must_use]
    pub fn shipped(width: u8, height: u8, seed: u64) -> Option<Self> {
        let rules = ServiceRules::shipped().ok()?;
        Self::new(rules, width, height, seed)
    }

    /// A city on a given table, or `None` when a side is zero or above [`MAX_SIDE`], or
    /// the table's walker cap is above [`MAX_WALKERS`].
    #[must_use]
    pub fn new(rules: ServiceRules<'static>, width: u8, height: u8, seed: u64) -> Option<Self> {
        if width == 0
            || height == 0
            || width > MAX_SIDE
            || height > MAX_SIDE
            || usize::from(rules.walk.walker_cap) > MAX_WALKERS
        {
            return None;
        }
        let cells = usize::from(width) * usize::from(height);
        let mut tile_rand = vec![0u8; cells];
        for (at, slot) in tile_rand.iter_mut().enumerate() {
            *slot = tile_random(seed, at);
        }
        Some(Self {
            rules,
            width,
            height,
            seed,
            tiles: vec![Tile::Empty; cells],
            pass: vec![0; cells],
            owner: vec![0; cells],
            tile_rand,
            buildings: Vec::with_capacity(MAX_BUILDINGS),
            walkers: [Walker::DEAD; MAX_WALKERS],
            dist: vec![FAR; cells],
            queue: vec![0; cells],
            tick_of_day: 0,
            day: 0,
            labour: Labour::default(),
            stats: Stats::default(),
        })
    }

    /// The table the city runs on.
    #[must_use]
    pub const fn rules(&self) -> &ServiceRules<'static> {
        &self.rules
    }

    /// The days that have passed.
    #[must_use]
    pub const fn day(&self) -> u32 {
        self.day
    }

    /// The labour pool as the last staffing left it.
    #[must_use]
    pub const fn labour(&self) -> Labour {
        self.labour
    }

    /// What the walkers have done so far.
    #[must_use]
    pub const fn stats(&self) -> Stats {
        self.stats
    }

    /// One building, or `None` when the slot is empty or out of range.
    #[must_use]
    pub fn building(&self, at: usize) -> Option<&Building> {
        self.buildings.get(at).filter(|building| building.used)
    }

    /// How many walkers are alive.
    #[must_use]
    pub fn walkers_alive(&self) -> usize {
        self.walkers.iter().filter(|walker| walker.alive).count()
    }

    /// Where a live walker stands, by slot.
    #[must_use]
    pub fn walker_at(&self, slot: usize) -> Option<(u8, u8)> {
        self.walkers
            .get(slot)
            .filter(|walker| walker.alive)
            .map(|walker| (walker.x, walker.y))
    }

    /// The sum of every buffer's capacity: it never moves once the city is made.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.tiles.capacity()
            + self.pass.capacity()
            + self.owner.capacity()
            + self.tile_rand.capacity()
            + self.buildings.capacity()
            + self.dist.capacity()
            + self.queue.capacity()
    }

    fn cell(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= i32::from(self.width) || y >= i32::from(self.height) {
            None
        } else {
            usize::try_from(y * i32::from(self.width) + x).ok()
        }
    }

    /// The ground at a tile, or `None` outside the city.
    #[must_use]
    pub fn tile(&self, x: u8, y: u8) -> Option<Tile> {
        self.cell(i32::from(x), i32::from(y))
            .map(|at| self.tiles[at])
    }

    /// Lay a road on an empty tile. False when the tile is outside, built on or a road.
    pub fn add_road(&mut self, x: u8, y: u8) -> bool {
        match self.cell(i32::from(x), i32::from(y)) {
            Some(at) if self.tiles[at] == Tile::Empty && self.owner[at] == 0 => {
                self.tiles[at] = Tile::Road;
                true
            }
            _ => false,
        }
    }

    /// Put a roadblock on a road. By default no kind of walker may roam through it.
    pub fn add_roadblock(&mut self, x: u8, y: u8) -> bool {
        match self.cell(i32::from(x), i32::from(y)) {
            Some(at) if self.tiles[at] == Tile::Road => {
                self.tiles[at] = Tile::Roadblock;
                self.pass[at] = 0;
                true
            }
            _ => false,
        }
    }

    /// Let (or stop) a kind of walker roaming through a roadblock (Augustus' roadblock
    /// permissions). False when the tile holds no roadblock.
    pub fn set_roadblock_pass(&mut self, x: u8, y: u8, service: Service, allow: bool) -> bool {
        match self.cell(i32::from(x), i32::from(y)) {
            Some(at) if self.tiles[at] == Tile::Roadblock => {
                let bit = 1u8 << service_code(service);
                if allow {
                    self.pass[at] |= bit;
                } else {
                    self.pass[at] &= !bit;
                }
                true
            }
            _ => false,
        }
    }

    /// Take a roadblock off its road.
    pub fn remove_roadblock(&mut self, x: u8, y: u8) -> bool {
        match self.cell(i32::from(x), i32::from(y)) {
            Some(at) if self.tiles[at] == Tile::Roadblock => {
                self.tiles[at] = Tile::Road;
                self.pass[at] = 0;
                true
            }
            _ => false,
        }
    }

    /// Build a house holding `people` (clamped to its level's room). Returns its slot,
    /// or `None` when the tile is taken or the city is full.
    pub fn add_house(&mut self, x: u8, y: u8, people: u16) -> Option<usize> {
        let room = self.rules.level(0).max_people;
        let slot = self.place(Kind::House, x, y, 1)?;
        self.buildings[slot].people = people.min(room);
        Some(slot)
    }

    /// Build a service building at its corner tile. Returns its slot, or `None` when the
    /// footprint is outside the city, taken or not bare ground, or the city is full.
    pub fn add_service(&mut self, service: Service, x: u8, y: u8) -> Option<usize> {
        let size = self.rules.rule(service).size;
        self.place(Kind::Service(service), x, y, size)
    }

    fn place(&mut self, kind: Kind, x: u8, y: u8, size: u8) -> Option<usize> {
        for dy in 0..i32::from(size) {
            for dx in 0..i32::from(size) {
                let at = self.cell(i32::from(x) + dx, i32::from(y) + dy)?;
                if self.tiles[at] != Tile::Empty || self.owner[at] != 0 {
                    return None;
                }
            }
        }
        let slot = if let Some(free) = self.buildings.iter().position(|b| !b.used) {
            free
        } else if self.buildings.len() < MAX_BUILDINGS {
            self.buildings.push(Building::EMPTY);
            self.buildings.len() - 1
        } else {
            return None;
        };
        self.buildings[slot] = Building {
            used: true,
            kind,
            x,
            y,
            size,
            ..Building::EMPTY
        };
        let tag = u16::try_from(slot + 1).ok()?;
        for dy in 0..i32::from(size) {
            for dx in 0..i32::from(size) {
                if let Some(at) = self.cell(i32::from(x) + dx, i32::from(y) + dy) {
                    self.owner[at] = tag;
                }
            }
        }
        Some(slot)
    }

    /// Pull a building down; its walker is gone with it. False when the slot is empty.
    pub fn remove(&mut self, slot: usize) -> bool {
        let Some(building) = self.buildings.get(slot).copied().filter(|b| b.used) else {
            return false;
        };
        for dy in 0..i32::from(building.size) {
            for dx in 0..i32::from(building.size) {
                if let Some(at) = self.cell(i32::from(building.x) + dx, i32::from(building.y) + dy)
                {
                    self.owner[at] = 0;
                }
            }
        }
        if building.walker > 0 {
            self.walkers[usize::from(building.walker) - 1].alive = false;
        }
        self.buildings[slot] = Building::EMPTY;
        true
    }

    /// The first road tile around a building, scanning row by row: where its walker
    /// comes out and where it goes home to (Julius `map_has_road_access`).
    fn access(&self, building: &Building) -> Option<(u8, u8)> {
        let size = i32::from(building.size);
        let (x, y) = (i32::from(building.x), i32::from(building.y));
        for yy in y - 1..=y + size {
            for xx in x - 1..=x + size {
                if let Some(at) = self.cell(xx, yy) {
                    if self.tiles[at] == Tile::Road {
                        return Some((u8::try_from(xx).ok()?, u8::try_from(yy).ok()?));
                    }
                }
            }
        }
        None
    }

    /// Run `ticks` ticks.
    pub fn run_ticks(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.tick();
        }
    }

    /// Run `days` whole days.
    pub fn run_days(&mut self, days: u32) {
        self.run_ticks(days * u32::from(self.rules.clock.ticks_per_day));
    }

    /// One tick: the chore due on this tick of the day, then every walker acts, then
    /// the clock moves on. Allocates nothing.
    pub fn tick(&mut self) {
        let clock = self.rules.clock;
        let now = self.tick_of_day;
        if now == clock.covered {
            self.decay_covered();
        }
        if now == clock.growth {
            self.grow_houses();
        }
        if now == clock.labour {
            self.staff_buildings();
        }
        if now == clock.water {
            self.mark_wells();
        }
        if now == clock.spawn {
            self.spawn_walkers();
        }
        if now == clock.decay {
            self.decay_coverage();
        }
        if now == clock.evolve {
            self.evolve_houses();
        }
        for slot in 0..usize::from(self.rules.walk.walker_cap) {
            if self.walkers[slot].alive {
                self.act(slot);
            }
        }
        self.tick_of_day += 1;
        if self.tick_of_day >= clock.ticks_per_day {
            self.tick_of_day = 0;
            self.day += 1;
        }
    }

    fn decay_covered(&mut self) {
        for building in &mut self.buildings {
            building.houses_covered = building.houses_covered.saturating_sub(1);
        }
    }

    fn decay_coverage(&mut self) {
        let step = self.rules.walk.decay_per_day;
        for building in &mut self.buildings {
            if building.used && building.kind == Kind::House {
                for value in &mut building.coverage {
                    *value = value.saturating_sub(step);
                }
            }
        }
    }

    fn grow_houses(&mut self) {
        let step = self.rules.people_per_day;
        for slot in 0..self.buildings.len() {
            let building = self.buildings[slot];
            if building.used && building.kind == Kind::House && self.access(&building).is_some() {
                let room = self.rules.level(usize::from(building.level)).max_people;
                let people = building.people.saturating_add(step).min(room);
                self.buildings[slot].people = people.max(building.people);
            }
        }
    }

    fn mark_wells(&mut self) {
        for building in &mut self.buildings {
            building.well = false;
        }
        for slot in 0..self.buildings.len() {
            let well = self.buildings[slot];
            if !well.used || well.kind != Kind::Service(Service::Well) {
                continue;
            }
            let radius = i32::from(self.rules.rule(Service::Well).radius);
            let size = i32::from(well.size);
            for yy in i32::from(well.y) - radius..i32::from(well.y) + size + radius {
                for xx in i32::from(well.x) - radius..i32::from(well.x) + size + radius {
                    if let Some(at) = self.cell(xx, yy) {
                        let id = usize::from(self.owner[at]);
                        if id > 0 && self.buildings[id - 1].kind == Kind::House {
                            self.buildings[id - 1].well = true;
                        }
                    }
                }
            }
        }
    }

    /// The city-wide labour pool (Augustus): working-age people anywhere staff
    /// buildings anywhere, by category.
    fn staff_buildings(&mut self) {
        let mut people = 0u32;
        let mut need = [0u32; CATEGORIES];
        for building in &self.buildings {
            if !building.used {
                continue;
            }
            match building.kind {
                Kind::House => people += u32::from(building.people),
                Kind::Service(service) => {
                    let rule = self.rules.rule(service);
                    need[usize::from(rule.category)] += u32::from(rule.laborers);
                }
            }
        }
        let available = people * u32::from(self.rules.working_age_percent) / 100;
        let needed: u32 = need.iter().sum();
        let mut given = need;
        if needed > available {
            given = [0; CATEGORIES];
            let mut left = available;
            let mut guard = 0;
            while left > 0 {
                guard += 1;
                if guard >= available {
                    break;
                }
                for category in ROUND_ROBIN {
                    let wanted = need[category] - given[category];
                    if wanted > 0 {
                        let give = u32::from(self.rules.quanta[category]).min(left).min(wanted);
                        given[category] += give;
                        left -= give;
                        if left == 0 {
                            break;
                        }
                    }
                }
            }
        }
        let mut employed = 0;
        for building in &mut self.buildings {
            if !building.used {
                continue;
            }
            if let Kind::Service(service) = building.kind {
                let rule = self.rules.rule(service);
                let category = usize::from(rule.category);
                let workers = given[category].min(u32::from(rule.laborers));
                given[category] -= workers;
                employed += workers;
                building.workers = u8::try_from(workers).unwrap_or(u8::MAX);
            }
        }
        self.labour = Labour {
            available,
            needed,
            employed,
            unemployed: available.saturating_sub(employed),
        };
    }

    fn spawn_walkers(&mut self) {
        for slot in 0..self.buildings.len() {
            let building = self.buildings[slot];
            let Kind::Service(service) = building.kind else {
                continue;
            };
            if !building.used || !service.walks() {
                continue;
            }
            // A building keeps one walker; a stale link is cleared (`has_figure_of_type`).
            if building.walker > 0 {
                if self.walkers[usize::from(building.walker) - 1].alive {
                    continue;
                }
                self.buildings[slot].walker = 0;
            }
            let rule = *self.rules.rule(service);
            let percent = u32::from(building.workers) * 100 / u32::from(rule.laborers);
            let Some(delay) = rule.delay(percent) else {
                continue;
            };
            let Some((x, y)) = self.access(&building) else {
                continue;
            };
            self.buildings[slot].spawn_delay = building.spawn_delay.saturating_add(1);
            let building = self.buildings[slot];
            if building.spawn_delay <= delay {
                continue;
            }
            let cap = usize::from(self.rules.walk.walker_cap);
            let Some(free) = self.walkers[..cap].iter().position(|walker| !walker.alive) else {
                self.stats.capped += 1;
                continue;
            };
            self.buildings[slot].spawn_delay = 0;
            let roam_dir = building.roam_dir & 3;
            self.buildings[slot].roam_dir = (roam_dir + 1) & 3;
            self.buildings[slot].walker = u8::try_from(free + 1).unwrap_or(0);
            self.walkers[free] = Walker {
                alive: true,
                service,
                building: u16::try_from(slot).unwrap_or(0),
                x,
                y,
                dir: roam_dir,
                moved: false,
                progress: self.rules.walk.ticks_per_tile - 1,
                roam: 0,
                phase: Phase::Roam,
                turn: if roam_dir & 1 == 0 { 1 } else { 3 },
                counter: 0,
            };
            self.stats.spawned += 1;
        }
    }

    fn evolve_houses(&mut self) {
        let grace = self.rules.devolve_grace;
        for slot in 0..self.buildings.len() {
            let house = self.buildings[slot];
            if !house.used || house.kind != Kind::House {
                continue;
            }
            let level = usize::from(house.level);
            let holds = self.meets(&house, level);
            let climbs =
                holds && house.people > 0 && level + 1 < LEVELS && self.meets(&house, level + 1);
            let falls = !holds && level > 0;
            let target = if falls {
                if house.devolve < grace {
                    self.buildings[slot].devolve += 1;
                    continue;
                }
                level - 1
            } else if climbs {
                level + 1
            } else {
                level
            };
            let room = self.rules.level(target).max_people;
            let next = &mut self.buildings[slot];
            next.devolve = 0;
            next.level = u8::try_from(target).unwrap_or(0);
            next.people = next.people.min(room);
        }
    }

    /// True when a house meets the needs of a level (Julius
    /// `has_required_goods_and_services`).
    fn meets(&self, house: &Building, level: usize) -> bool {
        let need = self.rules.level(level);
        (need.water == 0 || house.well)
            && (need.religion == 0 || house.coverage[1] > 0)
            && house.coverage[3] >= need.bathhouse
            && house.coverage[2] >= need.market
    }

    /// Walk one tick.
    fn act(&mut self, slot: usize) {
        let mut walker = self.walkers[slot];
        let home = usize::from(walker.building);
        let linked = self
            .buildings
            .get(home)
            .is_some_and(|building| building.used && usize::from(building.walker) == slot + 1);
        if !linked {
            self.walkers[slot].alive = false;
            return;
        }
        if walker.phase == Phase::Roam {
            walker.roam += 1;
            if walker.roam >= self.rules.rule(walker.service).roam_ticks {
                walker.phase = Phase::Home;
            }
        }
        walker.progress += 1;
        if walker.progress < self.rules.walk.ticks_per_tile {
            self.walkers[slot] = walker;
            return;
        }
        walker.progress = self.rules.walk.ticks_per_tile;
        let served = self.cover(&walker);
        let covered = &mut self.buildings[home].houses_covered;
        *covered = covered
            .saturating_add(served)
            .min(self.rules.walk.covered_cap);
        let step = if walker.phase == Phase::Roam {
            walker.counter = walker.counter.wrapping_add(1);
            let step = self.roam_step(&walker);
            if step.is_none() {
                walker.phase = Phase::Home;
            }
            step
        } else {
            let building = self.buildings[home];
            let Some(goal) = self.access(&building) else {
                self.lose(slot, home);
                return;
            };
            if (walker.x, walker.y) == goal {
                self.walkers[slot].alive = false;
                self.buildings[home].walker = 0;
                self.stats.home += 1;
                return;
            }
            let step = self.home_step(&walker, goal);
            if step.is_none() {
                self.lose(slot, home);
                return;
            }
            step
        };
        if let Some(dir) = step {
            let (dx, dy) = STEP[usize::from(dir)];
            walker.x = u8::try_from(i32::from(walker.x) + dx).unwrap_or(walker.x);
            walker.y = u8::try_from(i32::from(walker.y) + dy).unwrap_or(walker.y);
            walker.dir = dir;
            walker.moved = true;
            walker.progress = 0;
        }
        self.walkers[slot] = walker;
    }

    fn lose(&mut self, slot: usize, home: usize) {
        self.walkers[slot].alive = false;
        self.buildings[home].walker = 0;
        self.stats.lost += 1;
    }

    /// Set the coverage of every populated house within the radius of the walker's tile
    /// (Julius `provide_culture` over `map_grid_get_area(x, y, 1, 2)`); returns the count.
    fn cover(&mut self, walker: &Walker) -> u16 {
        let radius = i32::from(self.rules.walk.radius);
        let top = self.rules.walk.max_coverage;
        let mut served = 0;
        for yy in i32::from(walker.y) - radius..=i32::from(walker.y) + radius {
            for xx in i32::from(walker.x) - radius..=i32::from(walker.x) + radius {
                let Some(at) = self.cell(xx, yy) else {
                    continue;
                };
                let id = usize::from(self.owner[at]);
                if id == 0 {
                    continue;
                }
                let house = &mut self.buildings[id - 1];
                if house.kind == Kind::House && house.people > 0 {
                    house.coverage[usize::from(service_code(walker.service))] = top;
                    served += 1;
                }
            }
        }
        served
    }

    /// May a roaming walker of this service step onto the tile?
    fn roam_ok(&self, x: i32, y: i32, service: Service) -> bool {
        match self.cell(x, y) {
            Some(at) => match self.tiles[at] {
                Tile::Road => true,
                Tile::Roadblock => (self.pass[at] >> service_code(service)) & 1 == 1,
                Tile::Empty => false,
            },
            None => false,
        }
    }

    /// The next way for a roaming walker (Julius `figure_movement_roam_ticks`): none when
    /// no road is open; the only road when one is open (so a dead end turns it round);
    /// otherwise on from where it faces, or at a junction a pseudo-random pick, never
    /// back the way it came unless there is no other.
    fn roam_step(&self, walker: &Walker) -> Option<u8> {
        let (x, y) = (i32::from(walker.x), i32::from(walker.y));
        let mut open = [false; 4];
        let mut count = 0;
        for (dir, (dx, dy)) in STEP.iter().enumerate() {
            open[dir] = self.roam_ok(x + dx, y + dy, walker.service);
            count += usize::from(open[dir]);
        }
        let came = if walker.moved {
            (walker.dir + 2) & 3
        } else {
            NONE
        };
        if count == 0 {
            return None;
        }
        if count == 1 {
            return open
                .iter()
                .position(|&on| on)
                .and_then(|d| u8::try_from(d).ok());
        }
        let mut dir = if count == 2 {
            walker.dir & 3
        } else {
            let at = self.cell(x, y).unwrap_or(0);
            let mix = walker.counter.wrapping_add(u16::from(self.tile_rand[at]));
            u8::try_from((mix >> 1) & 3).unwrap_or(0)
        };
        for _ in 0..4 {
            if open[usize::from(dir)] && dir != came {
                return Some(dir);
            }
            dir = (dir + walker.turn) & 3;
        }
        open.iter()
            .position(|&on| on)
            .and_then(|d| u8::try_from(d).ok())
    }

    /// The next step of a walker going to a destination: down the distance field from
    /// the goal over every road, roadblocks included.
    fn home_step(&mut self, walker: &Walker, goal: (u8, u8)) -> Option<u8> {
        self.fill_distance(goal);
        let (x, y) = (i32::from(walker.x), i32::from(walker.y));
        let here = self.dist[self.cell(x, y)?];
        let mut best: Option<(u16, u8)> = None;
        for (dir, (dx, dy)) in STEP.iter().enumerate() {
            let Some(at) = self.cell(x + dx, y + dy) else {
                continue;
            };
            let far = self.dist[at];
            if far < here && best.is_none_or(|(near, _)| far < near) {
                best = Some((far, u8::try_from(dir).ok()?));
            }
        }
        best.map(|(_, dir)| dir)
    }

    /// Breadth-first distance from `goal` over roads and roadblocks, into the city's own
    /// buffers.
    fn fill_distance(&mut self, goal: (u8, u8)) {
        self.dist.fill(FAR);
        let Some(start) = self.cell(i32::from(goal.0), i32::from(goal.1)) else {
            return;
        };
        let (mut head, mut tail) = (0usize, 0usize);
        self.dist[start] = 0;
        self.queue[tail] = u16::try_from(start).unwrap_or(0);
        tail += 1;
        let width = i32::from(self.width);
        while head < tail {
            let at = usize::from(self.queue[head]);
            head += 1;
            let (x, y) = (
                i32::try_from(at).unwrap_or(0) % width,
                i32::try_from(at).unwrap_or(0) / width,
            );
            for (dx, dy) in STEP {
                let Some(next) = self.cell(x + dx, y + dy) else {
                    continue;
                };
                if self.tiles[next] != Tile::Empty && self.dist[next] == FAR {
                    self.dist[next] = self.dist[at] + 1;
                    self.queue[tail] = u16::try_from(next).unwrap_or(0);
                    tail += 1;
                }
            }
        }
    }

    /// A digest of the whole city: ground, buildings, walkers, labour and clock. Two
    /// cities built and run the same way agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/city/1");
        hasher.write_u64(self.seed);
        hasher.write_u8(self.width);
        hasher.write_u8(self.height);
        hasher.write_u32(self.day);
        hasher.write_u16(self.tick_of_day);
        for (tile, pass) in self.tiles.iter().zip(&self.pass) {
            hasher.write_u8(match tile {
                Tile::Empty => 0,
                Tile::Road => 1,
                Tile::Roadblock => 2,
            });
            hasher.write_u8(*pass);
        }
        for building in &self.buildings {
            hasher.write_bool(building.used);
            hasher.write_u8(building.kind.code());
            hasher.write(&[
                building.x,
                building.y,
                building.size,
                building.workers,
                building.spawn_delay,
                building.walker,
                building.roam_dir,
                building.level,
                u8::from(building.well),
                building.devolve,
            ]);
            hasher.write_u16(building.houses_covered);
            hasher.write_u16(building.people);
            hasher.write(&building.coverage);
        }
        for walker in &self.walkers {
            hasher.write_bool(walker.alive);
            if walker.alive {
                hasher.write(&[
                    service_code(walker.service),
                    walker.x,
                    walker.y,
                    walker.dir,
                    u8::from(walker.moved),
                    walker.progress,
                    u8::from(walker.phase == Phase::Home),
                    walker.turn,
                ]);
                hasher.write_u16(walker.building);
                hasher.write_u16(walker.roam);
                hasher.write_u16(walker.counter);
            }
        }
        for value in [
            self.labour.available,
            self.labour.needed,
            self.labour.employed,
            self.labour.unemployed,
            self.stats.spawned,
            self.stats.home,
            self.stats.lost,
            self.stats.capped,
        ] {
            hasher.write_u32(value);
        }
        hasher.finish()
    }
}

/// One byte of per-tile chance for the junction pick, fixed by the seed and the tile
/// (Julius `map_random_get`, a random grid made when the map is made).
fn tile_random(seed: u64, at: usize) -> u8 {
    let mut z =
        seed.wrapping_add((u64::try_from(at).unwrap_or(0) + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    u8::try_from((z ^ (z >> 31)) & 0xFF).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A city with a straight road along `row` from `from` to `to`.
    fn street(width: u8, height: u8, seed: u64, row: u8, from: u8, to: u8) -> City {
        let mut city = City::shipped(width, height, seed).expect("city");
        for x in from..=to {
            assert!(city.add_road(x, row));
        }
        city
    }

    fn cover_of(city: &City, house: usize, service: Service) -> u8 {
        city.building(house).expect("house").coverage(service)
    }

    #[test]
    fn the_shipped_table_loads_with_julius_numbers() {
        let rules = ServiceRules::shipped().expect("shipped service table");
        assert_eq!(rules.clock.ticks_per_day, 50, "50 ticks a day");
        assert_eq!(rules.walk.max_coverage, 96, "MAX_COVERAGE");
        assert_eq!(rules.walk.ticks_per_tile, 15);
        assert_eq!(rules.walk.radius, 2);
        assert_eq!(rules.rule(Service::Prefect).roam_ticks, 640);
        assert_eq!(rules.rule(Service::Temple).roam_ticks, 384);
        assert_eq!(rules.rule(Service::Prefect).delay(100), Some(0));
        assert_eq!(rules.rule(Service::Prefect).delay(50), Some(3));
        assert_eq!(rules.rule(Service::Prefect).delay(0), None);
        assert_eq!(rules.hash(), ServiceRules::shipped().expect("again").hash());
        assert!(
            ServiceRules::parse("[clock]\nticks_per_day = 50\n").is_err(),
            "a number with no ref is refused"
        );
    }

    #[test]
    fn a_prefect_covers_the_houses_by_the_road_and_the_cover_decays() {
        let mut city = street(24, 6, 1, 2, 0, 20);
        let prefecture = city
            .add_service(Service::Prefect, 0, 1)
            .expect("prefecture");
        let near = city.add_house(4, 1, 5).expect("house");
        let far = city.add_house(14, 1, 5).expect("house");
        assert_eq!(
            cover_of(&city, near, Service::Prefect),
            0,
            "nobody walked yet"
        );
        city.run_ticks(90);
        assert_eq!(
            city.stats().spawned,
            1,
            "the staffed prefecture sent a prefect"
        );
        assert!(
            cover_of(&city, near, Service::Prefect) >= 95,
            "the house by the road is covered: {}",
            cover_of(&city, near, Service::Prefect)
        );
        assert_eq!(
            cover_of(&city, far, Service::Prefect),
            0,
            "the walker has not reached the far house"
        );
        // The walker keeps passing, so the cover is kept up (one pass in ~10 days, 1 a day).
        let mut lowest = 96;
        for _ in 0..60 {
            city.run_days(1);
            lowest = lowest.min(cover_of(&city, near, Service::Prefect));
        }
        assert!(
            lowest >= 80,
            "kept while the walker passes: lowest {lowest}"
        );
        assert!(
            cover_of(&city, far, Service::Prefect) > 0,
            "the far one was reached too"
        );
        // Pull the prefecture down: nobody renews it, and it decays by one a day.
        assert!(city.remove(prefecture));
        city.run_days(1);
        let start = cover_of(&city, near, Service::Prefect);
        assert!(start > 0);
        city.run_days(10.min(u32::from(start)));
        let after = cover_of(&city, near, Service::Prefect);
        assert_eq!(after, start - 10.min(start), "one point a day");
        city.run_days(100);
        assert_eq!(
            cover_of(&city, near, Service::Prefect),
            0,
            "lost after decay"
        );
        assert_eq!(city.walkers_alive(), 0, "the walker went with its building");
    }

    #[test]
    fn a_roadblock_stops_a_roaming_walker() {
        let build = |block: bool| {
            let mut city = street(24, 6, 7, 2, 0, 20);
            city.add_service(Service::Prefect, 0, 1)
                .expect("prefecture");
            let near = city.add_house(4, 1, 5).expect("near");
            let far = city.add_house(14, 1, 5).expect("far");
            if block {
                assert!(city.add_roadblock(9, 2));
            }
            city.run_days(16);
            (
                cover_of(&city, near, Service::Prefect),
                cover_of(&city, far, Service::Prefect),
                city,
            )
        };
        let (near_open, far_open, _) = build(false);
        assert!(near_open > 0 && far_open > 0, "open street: both covered");
        let (near_shut, far_shut, shut) = build(true);
        assert!(near_shut > 0, "the near house is still covered");
        assert_eq!(far_shut, 0, "a roaming walker never got past the roadblock");
        assert!(shut.stats().spawned >= 1);
        // The permission Augustus gives a roadblock lets one kind of walker through.
        let mut city = street(24, 6, 7, 2, 0, 20);
        city.add_service(Service::Prefect, 0, 1)
            .expect("prefecture");
        city.add_house(4, 1, 5).expect("near");
        let far = city.add_house(14, 1, 5).expect("far");
        assert!(city.add_roadblock(9, 2));
        assert!(city.set_roadblock_pass(9, 2, Service::Prefect, true));
        city.run_days(16);
        assert!(
            cover_of(&city, far, Service::Prefect) > 0,
            "a permitted prefect passes"
        );
    }

    #[test]
    fn a_roadblock_does_not_stop_a_walker_going_home() {
        let mut city = street(24, 6, 3, 2, 0, 20);
        city.add_service(Service::Prefect, 0, 1)
            .expect("prefecture");
        city.add_house(4, 1, 5).expect("a");
        city.add_house(14, 1, 5).expect("b");
        // Let the walker go well past x = 9, then close the road behind it.
        let mut guard = 0;
        while city.walker_at(0).is_none_or(|(x, _)| x < 12) {
            city.tick();
            guard += 1;
            assert!(guard < 2000, "the walker went east");
        }
        assert!(city.add_roadblock(9, 2));
        // It cannot roam back through the block, but it is bound home once its roam is
        // spent, and a destination walker passes: it goes in and is not lost.
        let before = city.stats();
        city.run_ticks(900);
        let after = city.stats();
        assert_eq!(after.lost, before.lost, "nobody was lost");
        assert!(
            after.home > before.home,
            "the prefect walked home through the block"
        );
    }

    #[test]
    fn an_unstaffed_building_spawns_nothing() {
        let mut city = street(24, 6, 1, 2, 0, 20);
        city.add_service(Service::Prefect, 0, 1)
            .expect("prefecture");
        city.run_days(12);
        assert_eq!(city.labour().available, 0, "nobody lives here");
        assert_eq!(city.stats().spawned, 0, "no workers, no walker");
        assert_eq!(city.walkers_alive(), 0);
        // People move in: 10 people make 6 working-age workers, the prefecture's six.
        city.add_house(4, 1, 5).expect("a");
        city.add_house(5, 1, 5).expect("b");
        city.run_days(1);
        assert_eq!(city.labour().available, 6);
        assert!(city.stats().spawned >= 1, "now it sends a prefect");
    }

    #[test]
    fn half_staffing_waits_three_days_between_walkers() {
        let mut city = street(24, 6, 1, 2, 0, 20);
        city.add_service(Service::Prefect, 0, 1)
            .expect("prefecture");
        city.add_house(4, 1, 5)
            .expect("one house, three workers of six");
        city.run_ticks(3 * 50);
        let workers = city.building(0).expect("prefecture").workers();
        assert!(
            (3..=4).contains(&workers),
            "three or four of six: {workers}"
        );
        assert_eq!(city.stats().spawned, 0, "delay 3 at 50 percent");
        city.run_ticks(40);
        assert_eq!(city.stats().spawned, 1, "the fourth day's spawn");
    }

    #[test]
    fn houses_evolve_by_coverage_and_devolve_without_it() {
        let mut city = street(24, 8, 5, 3, 0, 14);
        city.add_service(Service::Well, 4, 2).expect("well");
        let temple = city.add_service(Service::Temple, 8, 1).expect("temple");
        city.add_service(Service::Market, 11, 1).expect("market");
        let houses = [
            city.add_house(5, 2, 5).expect("a"),
            city.add_house(6, 2, 5).expect("b"),
            city.add_house(3, 2, 5).expect("c"),
        ];
        assert_eq!(city.building(houses[1]).expect("b").level(), 0);
        city.run_days(60);
        for house in houses {
            let house = city.building(house).expect("house");
            assert!(house.has_well());
            assert_eq!(
                house.level(),
                4,
                "well, temple and market make a small hovel"
            );
        }
        // The temple goes: nobody renews the cover, and after it is spent the houses fall.
        assert!(city.remove(temple));
        city.run_days(110);
        for house in houses {
            let house = city.building(house).expect("house");
            assert_eq!(house.coverage(Service::Temple), 0);
            assert_eq!(house.level(), 3, "devolved one step to a large shack");
        }
    }

    #[test]
    fn a_short_labour_pool_is_shared_out_by_the_round_robin() {
        // 6 working-age workers, a prefecture (6, prefectures) and a market (5, commerce).
        let mut city = street(24, 6, 1, 2, 0, 20);
        city.add_service(Service::Market, 8, 0).expect("market");
        city.add_service(Service::Prefect, 0, 1)
            .expect("prefecture");
        city.add_house(4, 1, 5).expect("a");
        city.add_house(5, 1, 5).expect("b");
        city.run_days(1);
        let labour = city.labour();
        assert_eq!((labour.available, labour.needed), (6, 11));
        assert_eq!(labour.employed, 6, "everyone works");
        let prefecture = city.building(1).expect("prefecture").workers();
        let market = city.building(0).expect("market").workers();
        assert_eq!(prefecture + market, 6);
        assert_eq!(
            (prefecture, market),
            (4, 2),
            "pass one: prefectures 3, commerce 2; pass two: prefectures the last one"
        );
    }

    #[test]
    fn the_walker_cap_holds() {
        let mut rules = ServiceRules::shipped().expect("table");
        rules.walk.walker_cap = 2;
        let mut city = City::new(rules, 40, 6, 9).expect("city");
        for x in 0..30 {
            assert!(city.add_road(x, 2));
        }
        for x in [0u8, 5, 10, 15, 20] {
            city.add_service(Service::Prefect, x, 1)
                .expect("prefecture");
        }
        for x in 0..12 {
            city.add_house(x, 3, 5).expect("house");
        }
        for _ in 0..(3 * 50) {
            city.tick();
            assert!(city.walkers_alive() <= 2, "never above the cap");
        }
        assert_eq!(city.walkers_alive(), 2);
        assert!(city.stats().capped > 0, "the rest were turned away");
    }

    fn lattice(seed: u64) -> City {
        let mut city = City::shipped(20, 20, seed).expect("city");
        for i in 0..19 {
            for line in [3u8, 9, 15] {
                city.add_road(i, line);
                city.add_road(line, i);
            }
        }
        city.add_service(Service::Prefect, 4, 4)
            .expect("prefecture");
        city.add_service(Service::Temple, 10, 10).expect("temple");
        for (x, y) in [
            (5, 6),
            (6, 6),
            (7, 6),
            (11, 8),
            (12, 8),
            (13, 8),
            (4, 10),
            (4, 11),
        ] {
            city.add_house(x, y, 5);
        }
        city
    }

    #[test]
    fn the_same_seed_gives_the_same_city() {
        let (mut a, mut b) = (lattice(11), lattice(11));
        a.run_days(40);
        b.run_days(40);
        assert_eq!(a.hash(), b.hash(), "same seed, same hash");
        assert!(a.stats().spawned > 1, "the walkers walked");
        let mut other = lattice(12);
        other.run_days(40);
        assert_ne!(a.hash(), other.hash(), "another seed picks other turns");
    }

    #[test]
    fn the_tick_never_grows_a_buffer() {
        let mut city = lattice(2);
        let capacity = city.capacity();
        city.run_days(30);
        assert_eq!(city.capacity(), capacity, "no buffer was reallocated");
        assert!(city.walkers_alive() <= MAX_WALKERS);
    }
}
