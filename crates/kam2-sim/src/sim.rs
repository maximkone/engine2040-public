//! The tick: commands in, one deterministic step, one hash.

use crate::economy::{
    House, HouseKind, Stock, Ware, BAKERY_PERIOD, FARM_PERIOD, MILL_PERIOD, WOODCUTTER_PERIOD,
};
use crate::map::{Map, Terrain};
use crate::units::{find_path, step, unit_at, Shot, Unit, UnitKind};

/// What the player may do between ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Lay one road tile.
    PlaceRoad {
        /// Tile.
        x: i32,
        /// Tile.
        y: i32,
    },
    /// Place a house whose footprint must be free and must touch a road.
    PlaceHouse {
        /// What to build.
        kind: HouseKind,
        /// Top-left tile.
        x: i32,
        /// Top-left tile.
        y: i32,
    },
    /// Field one unit on open ground for its wares cost.
    SpawnUnit {
        /// What to field.
        kind: UnitKind,
        /// Team 0 or 1.
        side: u8,
        /// Tile.
        x: i32,
        /// Tile.
        y: i32,
    },
    /// Walk to a tile by the shortest open path; this clears the attack order.
    MoveUnit {
        /// Unit.
        id: u32,
        /// Tile.
        x: i32,
        /// Tile.
        y: i32,
    },
    /// Hold position and strike the target whenever it is in reach.
    Attack {
        /// Unit that strikes.
        id: u32,
        /// Unit that takes the hits.
        target: u32,
    },
}

/// Why a command was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The footprint leaves the map.
    OutOfBounds,
    /// Water, rock or an existing road is in the way.
    NotBuildable,
    /// Another house or unit stands there.
    Occupied,
    /// The footprint touches no road.
    NoRoad,
    /// The stock cannot pay the house's or the unit's cost.
    NotAffordable,
    /// The side is not 0 or 1.
    BadSide,
    /// No unit has this id, or it is the unit itself.
    NoUnit,
    /// No open path joins the unit and the tile.
    NoPath,
}

#[derive(Clone, Debug)]
struct Delivery {
    due: u32,
    seq: u64,
    ware: Ware,
}

/// One match: a map, houses, a stock, the deliveries in flight and the army.
#[derive(Clone, Debug)]
pub struct Sim {
    tick: u32,
    map: Map,
    houses: Vec<House>,
    stock: Stock,
    deliveries: Vec<Delivery>,
    seq: u64,
    units: Vec<Unit>,
    shots: Vec<Shot>,
    next_unit: u32,
}

impl Sim {
    /// A quiet map of `w` by `h` grass tiles.
    #[must_use]
    pub fn new(w: i32, h: i32) -> Self {
        Self {
            tick: 0,
            map: Map::new(w, h),
            houses: Vec::new(),
            stock: starting_stock(),
            deliveries: Vec::new(),
            seq: 0,
            units: Vec::new(),
            shots: Vec::new(),
            next_unit: 0,
        }
    }

    /// Read-only map.
    #[must_use]
    pub const fn map(&self) -> &Map {
        &self.map
    }

    /// Huts placed so far, in id order.
    #[must_use]
    pub fn houses(&self) -> &[House] {
        &self.houses
    }

    /// Units on the map, in spawn order.
    #[must_use]
    pub fn units(&self) -> &[Unit] {
        &self.units
    }

    /// One unit by id, if it lives.
    #[must_use]
    pub fn unit(&self, id: u32) -> Option<&Unit> {
        self.units.iter().find(|u| u.id == id)
    }

    /// Arrows in flight, in launch order.
    #[must_use]
    pub fn shots(&self) -> &[Shot] {
        &self.shots
    }

    /// Ticks played.
    #[must_use]
    pub const fn ticks(&self) -> u32 {
        self.tick
    }

    /// Scenario setup: paint terrain before the match starts.
    pub fn set_terrain(&mut self, x: i32, y: i32, terrain: Terrain) -> bool {
        self.map.set_terrain(x, y, terrain)
    }

    /// How many of one ware the town holds.
    #[must_use]
    pub fn stock(&self, ware: Ware) -> u32 {
        self.stock.get(ware)
    }

    /// Apply one command. Refused commands change nothing.
    pub fn apply(&mut self, cmd: Command) -> Result<(), Refusal> {
        match cmd {
            Command::PlaceRoad { x, y } => {
                if self.map.terrain(x, y).is_none() {
                    return Err(Refusal::OutOfBounds);
                }
                if self.map.is_road(x, y) || self.map.occupant(x, y).is_some() {
                    return Err(Refusal::Occupied);
                }
                if !self.map.lay_road(x, y) {
                    return Err(Refusal::NotBuildable);
                }
                self.relink();
                Ok(())
            }
            Command::PlaceHouse { kind, x, y } => {
                let (w, h) = kind.footprint();
                if x < 0 || y < 0 || x + w > self.map.width() || y + h > self.map.height() {
                    return Err(Refusal::OutOfBounds);
                }
                if self
                    .units
                    .iter()
                    .any(|u| u.x >= x && u.x < x + w && u.y >= y && u.y < y + h)
                {
                    return Err(Refusal::Occupied);
                }
                if !self.map.can_place(x, y, w, h) {
                    return Err(Refusal::NotBuildable);
                }
                if !self.map.touches_road(x, y, w, h) {
                    return Err(Refusal::NoRoad);
                }
                if !kind
                    .cost()
                    .iter()
                    .all(|(ware, n)| self.stock.get(*ware) >= *n)
                {
                    return Err(Refusal::NotAffordable);
                }
                for (ware, n) in kind.cost() {
                    self.stock.take(*ware, *n);
                }
                let id = u32::try_from(self.houses.len()).unwrap_or(u32::MAX);
                self.map.place(id, x, y, w, h);
                self.houses.push(House {
                    id,
                    kind,
                    x,
                    y,
                    link: None,
                    built_at: self.tick.saturating_add(kind.build_ticks()),
                });
                self.relink();
                Ok(())
            }
            Command::SpawnUnit { kind, side, x, y } => {
                if self.map.terrain(x, y).is_none() {
                    return Err(Refusal::OutOfBounds);
                }
                if side > 1 {
                    return Err(Refusal::BadSide);
                }
                if self.map.occupant(x, y).is_some() || unit_at(&self.units, x, y).is_some() {
                    return Err(Refusal::Occupied);
                }
                if !self.map.walkable(x, y) {
                    return Err(Refusal::NotBuildable);
                }
                if !kind
                    .cost()
                    .iter()
                    .all(|(ware, n)| self.stock.get(*ware) >= *n)
                {
                    return Err(Refusal::NotAffordable);
                }
                for (ware, n) in kind.cost() {
                    self.stock.take(*ware, *n);
                }
                let id = self.next_unit;
                self.next_unit = self.next_unit.saturating_add(1);
                self.units.push(Unit {
                    id,
                    kind,
                    side,
                    x,
                    y,
                    hp: kind.hp(),
                    reload: 0,
                    target: None,
                    path: std::collections::VecDeque::new(),
                    progress: 0,
                });
                Ok(())
            }
            Command::MoveUnit { id, x, y } => {
                let Some(i) = self.unit_index(id) else {
                    return Err(Refusal::NoUnit);
                };
                if self.map.terrain(x, y).is_none() {
                    return Err(Refusal::OutOfBounds);
                }
                if self.map.occupant(x, y).is_some() {
                    return Err(Refusal::Occupied);
                }
                if !self.map.walkable(x, y) {
                    return Err(Refusal::NotBuildable);
                }
                if unit_at(&self.units, x, y).is_some_and(|other| other != id) {
                    return Err(Refusal::Occupied);
                }
                let from = (self.units[i].x, self.units[i].y);
                let path = if from == (x, y) {
                    std::collections::VecDeque::new()
                } else {
                    match find_path(&self.map, &self.units, from, (x, y), id) {
                        Some(path) => path,
                        None => return Err(Refusal::NoPath),
                    }
                };
                self.units[i].path = path;
                self.units[i].progress = 0;
                self.units[i].target = None;
                Ok(())
            }
            Command::Attack { id, target } => {
                let Some(i) = self.unit_index(id) else {
                    return Err(Refusal::NoUnit);
                };
                if id == target || self.unit_index(target).is_none() {
                    return Err(Refusal::NoUnit);
                }
                self.units[i].target = Some(target);
                self.units[i].path.clear();
                self.units[i].progress = 0;
                Ok(())
            }
        }
    }

    fn unit_index(&self, id: u32) -> Option<usize> {
        self.units.iter().position(|u| u.id == id)
    }

    fn relink(&mut self) {
        let warehouses: Vec<(i32, i32, i32, i32)> = self
            .houses
            .iter()
            .filter(|h| h.kind == HouseKind::Warehouse)
            .map(|h| {
                let (w, hh) = h.kind.footprint();
                (h.x, h.y, w, hh)
            })
            .collect();
        for house in &mut self.houses {
            let (w, h) = house.kind.footprint();
            let from = (house.x, house.y, w, h);
            house.link = warehouses
                .iter()
                .filter_map(|to| self.map.road_distance(from, *to))
                .min();
        }
    }

    fn deliver(&mut self, ware: Ware, distance: u32) {
        self.seq = self.seq.wrapping_add(1);
        self.deliveries.push(Delivery {
            due: self.tick.saturating_add(distance),
            seq: self.seq,
            ware,
        });
    }

    /// Play one tick.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.land();
        let plan: Vec<(u32, HouseKind, u32, u32)> = self
            .houses
            .iter()
            .filter_map(|h| h.link.map(|link| (h.id, h.kind, link, h.built_at)))
            .collect();
        for (id, kind, link, built_at) in plan {
            if self.tick < built_at {
                continue;
            }
            let phase = self.tick.wrapping_add(id);
            match kind {
                HouseKind::Warehouse => {}
                HouseKind::Farm => {
                    if phase % FARM_PERIOD == 0 {
                        self.deliver(Ware::Grain, link);
                    }
                }
                HouseKind::Mill => {
                    if phase % MILL_PERIOD == 0 && self.stock.take(Ware::Grain, 1) {
                        self.deliver(Ware::Flour, link);
                    }
                }
                HouseKind::Bakery => {
                    if phase % BAKERY_PERIOD == 0 && self.stock.take(Ware::Flour, 1) {
                        self.deliver(Ware::Bread, link);
                    }
                }
                HouseKind::Woodcutter => {
                    if phase % WOODCUTTER_PERIOD == 0 {
                        self.deliver(Ware::Wood, link);
                    }
                }
            }
        }
        step(self.tick, &self.map, &mut self.units, &mut self.shots);
    }

    fn land(&mut self) {
        let now = self.tick;
        let mut keep = Vec::with_capacity(self.deliveries.len());
        for d in self.deliveries.drain(..) {
            if d.due <= now {
                self.stock.add(d.ware, 1);
            } else {
                keep.push(d);
            }
        }
        self.deliveries = keep;
    }

    /// State digest. Same commands plus same ticks give the same number.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        h = mix(h, u64::from(self.tick));
        h = mix(h, u64::from(self.map.width().unsigned_abs()));
        h = mix(h, u64::from(self.map.height().unsigned_abs()));
        for y in 0..self.map.height() {
            for x in 0..self.map.width() {
                let t = self.map.terrain(x, y).map_or(0, |t| match t {
                    crate::map::Terrain::Grass => 1,
                    crate::map::Terrain::Dirt => 2,
                    crate::map::Terrain::Rock => 3,
                    crate::map::Terrain::Water => 4,
                });
                h = mix(h, t);
                h = mix(h, u64::from(self.map.is_road(x, y)));
                h = mix(h, u64::from(self.map.occupant(x, y).unwrap_or(u32::MAX)));
            }
        }
        for house in &self.houses {
            h = mix(h, u64::from(house.id));
            h = mix(h, u64::from(house.kind.index()));
            h = mix(h, u64::from(house.link.unwrap_or(u32::MAX)));
            h = mix(h, u64::from(house.built_at));
        }
        for ware in Ware::ALL {
            h = mix(h, u64::from(self.stock.get(ware)));
        }
        for d in &self.deliveries {
            h = mix(h, u64::from(d.due));
            h = mix(h, d.seq);
            h = mix(h, u64::from(d.ware.index() as u32));
        }
        h = mix(h, u64::from(self.next_unit));
        for unit in &self.units {
            h = mix(h, u64::from(unit.id));
            h = mix(h, u64::from(unit.kind.index()));
            h = mix(h, u64::from(unit.side));
            h = mix(h, u64::from(unit.x.cast_unsigned()));
            h = mix(h, u64::from(unit.y.cast_unsigned()));
            h = mix(h, u64::from(unit.hp));
            h = mix(h, u64::from(unit.reload));
            h = mix(h, u64::from(unit.target.unwrap_or(u32::MAX)));
            h = mix(h, u64::from(unit.progress));
            for (px, py) in &unit.path {
                h = mix(h, u64::from(px.cast_unsigned()));
                h = mix(h, u64::from(py.cast_unsigned()));
            }
        }
        for shot in &self.shots {
            h = mix(h, u64::from(shot.from));
            h = mix(h, u64::from(shot.target));
            h = mix(h, u64::from(shot.due));
            h = mix(h, u64::from(shot.damage));
        }
        h
    }
}

fn starting_stock() -> Stock {
    let mut stock = Stock::default();
    stock.add(Ware::Wood, 40);
    stock.add(Ware::Stone, 20);
    stock
}

fn mix(mut h: u64, v: u64) -> u64 {
    for b in v.to_le_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::{Command, Refusal, Sim};
    use crate::economy::{HouseKind, Ware};
    use crate::map::Terrain;

    fn town() -> Sim {
        let mut sim = Sim::new(16, 12);
        for x in 0..14 {
            sim.apply(Command::PlaceRoad { x, y: 6 }).expect("road");
        }
        sim.apply(Command::PlaceHouse {
            kind: HouseKind::Warehouse,
            x: 0,
            y: 3,
        })
        .expect("warehouse");
        sim.apply(Command::PlaceHouse {
            kind: HouseKind::Farm,
            x: 4,
            y: 4,
        })
        .expect("farm");
        sim.apply(Command::PlaceHouse {
            kind: HouseKind::Mill,
            x: 7,
            y: 4,
        })
        .expect("mill");
        sim.apply(Command::PlaceHouse {
            kind: HouseKind::Bakery,
            x: 10,
            y: 4,
        })
        .expect("bakery");
        sim
    }

    #[test]
    fn a_town_bakes_bread() {
        let mut sim = town();
        for _ in 0..900 {
            sim.tick();
        }
        assert!(
            sim.stock(Ware::Bread) > 0,
            "bread {}",
            sim.stock(Ware::Bread)
        );
        assert_eq!(sim.stock(Ware::Grain), 0);
        assert_eq!(sim.stock(Ware::Flour), 0);
    }

    #[test]
    fn same_commands_same_hash() {
        let mut a = town();
        let mut b = town();
        for _ in 0..600 {
            a.tick();
            b.tick();
        }
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn no_road_no_bread() {
        let mut sim = Sim::new(16, 12);
        assert_eq!(
            sim.apply(Command::PlaceHouse {
                kind: HouseKind::Bakery,
                x: 2,
                y: 2
            }),
            Err(Refusal::NoRoad)
        );
    }

    #[test]
    fn a_new_house_spends_wood_and_waits_for_its_build_time() {
        let mut sim = Sim::new(10, 10);
        for x in 0..6 {
            sim.apply(Command::PlaceRoad { x, y: 3 }).expect("road");
        }
        sim.apply(Command::PlaceHouse {
            kind: HouseKind::Warehouse,
            x: 0,
            y: 0,
        })
        .expect("warehouse");
        sim.apply(Command::PlaceHouse {
            kind: HouseKind::Woodcutter,
            x: 3,
            y: 1,
        })
        .expect("woodcutter");
        assert_eq!(sim.stock(Ware::Wood), 28, "10 plus 2 wood spent");
        for _ in 0..19 {
            sim.tick();
        }
        assert_eq!(sim.stock(Ware::Wood), 28, "nothing before the build ends");
        for _ in 0..60 {
            sim.tick();
        }
        assert!(sim.stock(Ware::Wood) > 28, "wood {}", sim.stock(Ware::Wood));
    }

    #[test]
    fn a_house_that_cannot_be_paid_is_refused() {
        let mut sim = Sim::new(16, 12);
        for x in 0..15 {
            sim.apply(Command::PlaceRoad { x, y: 6 }).expect("road");
        }
        for x in [0, 2, 4, 6, 8] {
            sim.apply(Command::PlaceHouse {
                kind: HouseKind::Bakery,
                x,
                y: 4,
            })
            .expect("bakery");
        }
        assert_eq!(sim.stock(Ware::Wood), 0, "five bakeries ate forty wood");
        assert_eq!(
            sim.apply(Command::PlaceHouse {
                kind: HouseKind::Bakery,
                x: 12,
                y: 4
            }),
            Err(Refusal::NotAffordable)
        );
    }

    #[test]
    fn water_refuses_a_house() {
        let mut sim = Sim::new(8, 8);
        assert!(sim.set_terrain(2, 2, Terrain::Water));
        for x in 2..4 {
            sim.apply(Command::PlaceRoad { x, y: 4 }).expect("road");
        }
        assert_eq!(
            sim.apply(Command::PlaceHouse {
                kind: HouseKind::Bakery,
                x: 2,
                y: 2
            }),
            Err(Refusal::NotBuildable)
        );
        assert_eq!(
            sim.apply(Command::PlaceRoad { x: 99, y: 99 }),
            Err(Refusal::OutOfBounds)
        );
    }
}
