//! The tick: commands in, one deterministic step, one hash.
//!
//! Each side owns its stock, its houses and the deliveries those houses send, so one
//! side's wares never pay for the other's build or unit. Terrain and roads stay shared:
//! both sides build and walk on the same map.

use crate::economy::{
    House, HouseKind, Stock, Ware, BAKERY_PERIOD, FARM_PERIOD, MILL_PERIOD, WOODCUTTER_PERIOD,
};
use crate::map::{Map, Terrain};
use crate::units::{find_path, step, unit_at, Shot, Unit, UnitKind};

/// How many sides a match seats.
pub const SIDE_COUNT: usize = 2;

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
        /// Team 0 or 1; it must be the side the command is applied for.
        side: u8,
        /// Tile.
        x: i32,
        /// Tile.
        y: i32,
    },
    /// Walk to a tile by the shortest open path; this clears the attack order.
    MoveUnit {
        /// Unit of the side the command is applied for.
        id: u32,
        /// Tile.
        x: i32,
        /// Tile.
        y: i32,
    },
    /// Hold position and strike the target whenever it is in reach.
    Attack {
        /// Unit of the side the command is applied for.
        id: u32,
        /// Unit that takes the hits; it may belong to either side.
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
    /// The side is not 0 or 1, or a command names another side than the caller.
    BadSide,
    /// The unit belongs to the other side.
    NotYourUnit,
    /// No unit has this id, or it is the unit itself.
    NoUnit,
    /// No open path joins the unit and the tile.
    NoPath,
}

#[derive(Clone, Debug)]
struct Delivery {
    due: u32,
    seq: u64,
    side: u8,
    ware: Ware,
}

/// One match: a map, each side's houses and stock, the deliveries in flight and the army.
#[derive(Clone, Debug)]
pub struct Sim {
    tick: u32,
    map: Map,
    houses: Vec<House>,
    stocks: [Stock; SIDE_COUNT],
    deliveries: Vec<Delivery>,
    seq: u64,
    units: Vec<Unit>,
    shots: Vec<Shot>,
    next_unit: u32,
}

impl Sim {
    /// A quiet map of `w` by `h` grass tiles, with each side's starting stock.
    #[must_use]
    pub fn new(w: i32, h: i32) -> Self {
        Self {
            tick: 0,
            map: Map::new(w, h),
            houses: Vec::new(),
            stocks: [starting_stock(), starting_stock()],
            deliveries: Vec::new(),
            seq: 0,
            units: Vec::new(),
            shots: Vec::new(),
            next_unit: 0,
        }
    }

    /// Read-only map. Roads and terrain are shared by both sides.
    #[must_use]
    pub const fn map(&self) -> &Map {
        &self.map
    }

    /// Every hut placed, both sides, in id order.
    #[must_use]
    pub fn houses(&self) -> &[House] {
        &self.houses
    }

    /// The huts `side` owns, in placement order. Empty for a side that does not exist.
    pub fn houses_of(&self, side: u8) -> impl Iterator<Item = &House> + '_ {
        self.houses.iter().filter(move |house| house.side == side)
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

    /// How many of one ware side 0 holds.
    #[must_use]
    pub fn stock(&self, ware: Ware) -> u32 {
        self.stock_of(0, ware)
    }

    /// How many of one ware `side` holds; a side that does not exist holds nothing.
    #[must_use]
    pub fn stock_of(&self, side: u8, ware: Ware) -> u32 {
        self.stocks
            .get(usize::from(side))
            .map_or(0, |stock| stock.get(ware))
    }

    /// Apply one command for side 0. Refused commands change nothing.
    ///
    /// # Errors
    ///
    /// Returns the [`Refusal`] that stopped the command.
    pub fn apply(&mut self, cmd: Command) -> Result<(), Refusal> {
        self.apply_for(0, cmd)
    }

    /// Apply one command for `side`. Refused commands change nothing.
    ///
    /// Roads are shared terrain, so either side may lay one. A house or a unit spends
    /// only that side's stock, and a unit order reaches only that side's units.
    ///
    /// # Errors
    ///
    /// Returns the [`Refusal`] that stopped the command.
    pub fn apply_for(&mut self, side: u8, cmd: Command) -> Result<(), Refusal> {
        if usize::from(side) >= SIDE_COUNT {
            return Err(Refusal::BadSide);
        }
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
                let Some(stock) = self.stocks.get_mut(usize::from(side)) else {
                    return Err(Refusal::BadSide);
                };
                if !kind.cost().iter().all(|(ware, n)| stock.get(*ware) >= *n) {
                    return Err(Refusal::NotAffordable);
                }
                for (ware, n) in kind.cost() {
                    stock.take(*ware, *n);
                }
                let id = u32::try_from(self.houses.len()).unwrap_or(u32::MAX);
                self.map.place(id, x, y, w, h);
                self.houses.push(House {
                    id,
                    side,
                    kind,
                    x,
                    y,
                    link: None,
                    built_at: self.tick.saturating_add(kind.build_ticks()),
                });
                self.relink();
                Ok(())
            }
            Command::SpawnUnit {
                kind,
                side: unit_side,
                x,
                y,
            } => {
                if self.map.terrain(x, y).is_none() {
                    return Err(Refusal::OutOfBounds);
                }
                if unit_side != side {
                    return Err(Refusal::BadSide);
                }
                if self.map.occupant(x, y).is_some() || unit_at(&self.units, x, y).is_some() {
                    return Err(Refusal::Occupied);
                }
                if !self.map.walkable(x, y) {
                    return Err(Refusal::NotBuildable);
                }
                let Some(stock) = self.stocks.get_mut(usize::from(side)) else {
                    return Err(Refusal::BadSide);
                };
                if !kind.cost().iter().all(|(ware, n)| stock.get(*ware) >= *n) {
                    return Err(Refusal::NotAffordable);
                }
                for (ware, n) in kind.cost() {
                    stock.take(*ware, *n);
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
                if self.units[i].side != side {
                    return Err(Refusal::NotYourUnit);
                }
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
                if self.units[i].side != side {
                    return Err(Refusal::NotYourUnit);
                }
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
        let warehouses: Vec<(u8, i32, i32, i32, i32)> = self
            .houses
            .iter()
            .filter(|h| h.kind == HouseKind::Warehouse)
            .map(|h| {
                let (w, hh) = h.kind.footprint();
                (h.side, h.x, h.y, w, hh)
            })
            .collect();
        for house in &mut self.houses {
            let (w, h) = house.kind.footprint();
            let from = (house.x, house.y, w, h);
            house.link = warehouses
                .iter()
                .filter(|(side, ..)| *side == house.side)
                .filter_map(|(_, x, y, w, h)| self.map.road_distance(from, (*x, *y, *w, *h)))
                .min();
        }
    }

    fn deliver(&mut self, side: u8, ware: Ware, distance: u32) {
        self.seq = self.seq.wrapping_add(1);
        self.deliveries.push(Delivery {
            due: self.tick.saturating_add(distance),
            seq: self.seq,
            side,
            ware,
        });
    }

    /// Spend one ware from `side`'s stock; false when it has none.
    fn take(&mut self, side: u8, ware: Ware) -> bool {
        self.stocks
            .get_mut(usize::from(side))
            .is_some_and(|stock| stock.take(ware, 1))
    }

    /// Play one tick.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.land();
        let plan: Vec<(u32, u8, HouseKind, u32, u32)> = self
            .houses
            .iter()
            .filter_map(|h| h.link.map(|link| (h.id, h.side, h.kind, link, h.built_at)))
            .collect();
        for (id, side, kind, link, built_at) in plan {
            if self.tick < built_at {
                continue;
            }
            let phase = self.tick.wrapping_add(id);
            match kind {
                HouseKind::Warehouse => {}
                HouseKind::Farm => {
                    if phase % FARM_PERIOD == 0 {
                        self.deliver(side, Ware::Grain, link);
                    }
                }
                HouseKind::Mill => {
                    if phase % MILL_PERIOD == 0 && self.take(side, Ware::Grain) {
                        self.deliver(side, Ware::Flour, link);
                    }
                }
                HouseKind::Bakery => {
                    if phase % BAKERY_PERIOD == 0 && self.take(side, Ware::Flour) {
                        self.deliver(side, Ware::Bread, link);
                    }
                }
                HouseKind::Woodcutter => {
                    if phase % WOODCUTTER_PERIOD == 0 {
                        self.deliver(side, Ware::Wood, link);
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
                if let Some(stock) = self.stocks.get_mut(usize::from(d.side)) {
                    stock.add(d.ware, 1);
                }
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
            h = mix(h, u64::from(house.side));
            h = mix(h, u64::from(house.kind.index()));
            h = mix(h, u64::from(house.link.unwrap_or(u32::MAX)));
            h = mix(h, u64::from(house.built_at));
        }
        for side in 0..SIDE_COUNT {
            let Some(stock) = self.stocks.get(side) else {
                continue;
            };
            for ware in Ware::ALL {
                h = mix(h, u64::from(stock.get(ware)));
            }
        }
        for d in &self.deliveries {
            h = mix(h, u64::from(d.due));
            h = mix(h, d.seq);
            h = mix(h, u64::from(d.side));
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

    /// One road with a full chain on each side of it, both chains paid from its own purse.
    fn two_towns() -> Sim {
        let mut sim = Sim::new(28, 12);
        for x in 0..=26 {
            sim.apply(Command::PlaceRoad { x, y: 6 }).expect("road");
        }
        for (side, houses) in [
            (
                0u8,
                [
                    (HouseKind::Warehouse, 0, 3),
                    (HouseKind::Farm, 4, 4),
                    (HouseKind::Mill, 7, 4),
                    (HouseKind::Bakery, 10, 4),
                ],
            ),
            (
                1u8,
                [
                    (HouseKind::Warehouse, 14, 3),
                    (HouseKind::Farm, 18, 4),
                    (HouseKind::Mill, 21, 4),
                    (HouseKind::Bakery, 24, 4),
                ],
            ),
        ] {
            for (kind, x, y) in houses {
                sim.apply_for(side, Command::PlaceHouse { kind, x, y })
                    .expect("the chain fits its own purse");
            }
        }
        sim
    }

    #[test]
    fn each_side_builds_its_own_house_list() {
        let sim = two_towns();
        assert_eq!(sim.houses().len(), 8, "four houses on each side");
        let side0: Vec<HouseKind> = sim.houses_of(0).map(|house| house.kind).collect();
        let side1: Vec<HouseKind> = sim.houses_of(1).map(|house| house.kind).collect();
        let chain = [
            HouseKind::Warehouse,
            HouseKind::Farm,
            HouseKind::Mill,
            HouseKind::Bakery,
        ];
        assert_eq!(side0, chain, "side 0 holds its own chain");
        assert_eq!(side1, chain, "side 1 holds its own chain");
        assert!(sim.houses_of(2).next().is_none(), "only two sides exist");
        assert!(
            sim.houses_of(0).all(|house| house.side == 0)
                && sim.houses_of(1).all(|house| house.side == 1),
            "every house carries its owner"
        );
    }

    #[test]
    fn each_side_bakes_bread_from_its_own_stock() {
        let mut sim = two_towns();
        for _ in 0..900 {
            sim.tick();
        }
        assert!(
            sim.stock_of(0, Ware::Bread) > 0,
            "side 0 bread {}",
            sim.stock_of(0, Ware::Bread)
        );
        assert!(
            sim.stock_of(1, Ware::Bread) > 0,
            "side 1 bread {}",
            sim.stock_of(1, Ware::Bread)
        );
        assert!(
            sim.stock_of(0, Ware::Grain) <= 1,
            "side 0 grain waits at most for the next mill run: {}",
            sim.stock_of(0, Ware::Grain)
        );
        assert!(
            sim.stock_of(1, Ware::Grain) <= 1,
            "side 1 grain waits at most for the next mill run: {}",
            sim.stock_of(1, Ware::Grain)
        );
        assert!(
            sim.stock_of(0, Ware::Flour) <= 1,
            "side 0 flour waits at most for the next bake: {}",
            sim.stock_of(0, Ware::Flour)
        );
        assert!(
            sim.stock_of(1, Ware::Flour) <= 1,
            "side 1 flour waits at most for the next bake: {}",
            sim.stock_of(1, Ware::Flour)
        );
    }

    #[test]
    fn deliveries_land_on_the_owning_sides_warehouse() {
        let mut sim = Sim::new(28, 12);
        for x in 0..=26 {
            sim.apply(Command::PlaceRoad { x, y: 6 }).expect("road");
        }
        for (side, x) in [(0u8, 0), (1, 14)] {
            sim.apply_for(
                side,
                Command::PlaceHouse {
                    kind: HouseKind::Warehouse,
                    x,
                    y: 3,
                },
            )
            .expect("warehouse");
            sim.apply_for(
                side,
                Command::PlaceHouse {
                    kind: HouseKind::Farm,
                    x: x + 4,
                    y: 4,
                },
            )
            .expect("farm");
        }
        for _ in 0..100 {
            sim.tick();
        }
        assert!(sim.stock_of(0, Ware::Grain) > 0, "side 0 keeps its grain");
        assert!(sim.stock_of(1, Ware::Grain) > 0, "side 1 keeps its grain");
        assert_eq!(sim.stock_of(0, Ware::Bread), 0, "no bakery on side 0");
        assert_eq!(sim.stock_of(1, Ware::Bread), 0, "no bakery on side 1");
    }

    #[test]
    fn a_carrier_never_serves_the_other_side() {
        let mut sim = Sim::new(28, 12);
        for x in 0..=26 {
            sim.apply(Command::PlaceRoad { x, y: 6 }).expect("road");
        }
        sim.apply_for(
            0,
            Command::PlaceHouse {
                kind: HouseKind::Warehouse,
                x: 0,
                y: 3,
            },
        )
        .expect("near side-0 warehouse");
        sim.apply_for(
            1,
            Command::PlaceHouse {
                kind: HouseKind::Warehouse,
                x: 22,
                y: 3,
            },
        )
        .expect("far side-1 warehouse");
        sim.apply_for(
            1,
            Command::PlaceHouse {
                kind: HouseKind::Farm,
                x: 4,
                y: 4,
            },
        )
        .expect("side-1 farm beside the side-0 warehouse");
        let farm = sim
            .houses_of(1)
            .find(|house| house.kind == HouseKind::Farm)
            .expect("the farm stands");
        let near = sim
            .map()
            .road_distance((4, 4, 2, 2), (0, 3, 3, 3))
            .expect("the near road joins");
        let far = sim
            .map()
            .road_distance((4, 4, 2, 2), (22, 3, 3, 3))
            .expect("the far road joins");
        assert!(near < far, "the other side's warehouse is the closer one");
        assert_eq!(farm.link, Some(far), "the side-1 farm links to side 1");
        for _ in 0..100 {
            sim.tick();
        }
        assert!(
            sim.stock_of(1, Ware::Grain) > 0,
            "the side-1 carrier landed its grain"
        );
        assert_eq!(
            sim.stock_of(0, Ware::Grain),
            0,
            "side 0 never received side 1's grain"
        );
    }

    #[test]
    fn side_zero_bread_never_funds_side_one() {
        let mut sim = Sim::new(28, 12);
        for x in 0..=26 {
            sim.apply(Command::PlaceRoad { x, y: 6 }).expect("road");
        }
        for (kind, x, y) in [
            (HouseKind::Warehouse, 0, 3),
            (HouseKind::Farm, 4, 4),
            (HouseKind::Mill, 7, 4),
            (HouseKind::Bakery, 10, 4),
        ] {
            sim.apply_for(0, Command::PlaceHouse { kind, x, y })
                .expect("side 0 pays from its own purse");
        }
        assert_eq!(
            sim.stock_of(0, Ware::Wood),
            12,
            "side 0 wood after its chain"
        );
        assert_eq!(sim.stock_of(1, Ware::Wood), 40, "side 1 wood untouched");
        for (kind, x, y) in [
            (HouseKind::Warehouse, 14, 3),
            (HouseKind::Bakery, 18, 4),
            (HouseKind::Bakery, 20, 4),
            (HouseKind::Bakery, 22, 4),
        ] {
            sim.apply_for(1, Command::PlaceHouse { kind, x, y })
                .expect("side 1 pays from its own purse");
        }
        assert_eq!(
            sim.stock_of(1, Ware::Wood),
            6,
            "side 1 wood after its own buys"
        );
        assert_eq!(
            sim.stock_of(0, Ware::Wood),
            12,
            "side 1 spent none of side 0's"
        );
        assert_eq!(
            sim.apply_for(
                1,
                Command::PlaceHouse {
                    kind: HouseKind::Bakery,
                    x: 24,
                    y: 4
                }
            ),
            Err(Refusal::NotAffordable),
            "side 1 cannot reach side 0's wood"
        );
        for _ in 0..900 {
            sim.tick();
        }
        assert!(sim.stock_of(0, Ware::Bread) > 0, "side 0 bakes");
        assert_eq!(
            sim.stock_of(1, Ware::Bread),
            0,
            "side 1 gets no side-0 bread"
        );
    }

    #[test]
    fn commands_reach_only_their_own_sides_units() {
        let mut sim = Sim::new(10, 10);
        sim.apply_for(
            0,
            Command::SpawnUnit {
                kind: crate::units::UnitKind::Worker,
                side: 0,
                x: 2,
                y: 2,
            },
        )
        .expect("side 0 fields a worker");
        let id = sim.units()[0].id;
        assert_eq!(
            sim.apply_for(1, Command::MoveUnit { id, x: 3, y: 2 }),
            Err(Refusal::NotYourUnit)
        );
        assert_eq!(
            sim.apply_for(1, Command::Attack { id, target: 99 }),
            Err(Refusal::NotYourUnit)
        );
        assert_eq!(
            sim.apply_for(
                1,
                Command::SpawnUnit {
                    kind: crate::units::UnitKind::Worker,
                    side: 0,
                    x: 5,
                    y: 5
                }
            ),
            Err(Refusal::BadSide),
            "a side-1 hand cannot field a side-0 unit"
        );
        assert_eq!(
            sim.apply_for(2, Command::PlaceRoad { x: 0, y: 0 }),
            Err(Refusal::BadSide)
        );
        sim.apply_for(0, Command::MoveUnit { id, x: 3, y: 2 })
            .expect("the owner moves it");
        assert_eq!(sim.stock_of(1, Ware::Wood), 40, "side 1 paid nothing");
        assert_eq!(sim.stock_of(0, Ware::Wood), 39, "side 0 paid one wood");
    }
}
