//! The AI mayor: one decision round, read from the sim and written as commands.
//!
//! `Mayor::decide` is a pure function of the state: no clock, no IO, no floating point.
//! It checks every command on a private copy of the sim, in the order it returns them, so
//! the list never asks for something the real sim refuses. Roads come first, then houses
//! (warehouse, then the chain the stock asks for), then one swordsman; house and unit ids
//! are handed out in exactly that order. The seat defaults to side 0; [`Mayor::with_side`]
//! moves it to the other team. It reads and spends only its own side's stock and house
//! list, and its army counts only that side's swordsmen. Apply its commands with
//! [`Sim::apply_for`] and [`Mayor::side`].

use crate::economy::{HouseKind, Ware};
use crate::sim::{Command, Sim};
use crate::units::UnitKind;

/// Road tiles one round may lay.
const ROADS_PER_ROUND: usize = 4;
/// Houses one round may place.
const HOUSES_PER_ROUND: usize = 2;
/// Houses of one chain kind the town grows to.
const HOUSES_PER_KIND: u32 = 3;
/// Woodcutters the town keeps at most.
const WOODCUTTERS: u32 = 2;

/// A city mayor: lay roads toward a target site, build the chain, field swordsmen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mayor {
    target: (i32, i32),
    side: u8,
    wood_reserve: u32,
    stone_reserve: u32,
    army_wood: u32,
    army_cap: u32,
}

impl Mayor {
    /// A mayor whose road ends at `target`; the road starts at the map's left edge on the
    /// target's row, so the site must sit on open ground reachable along that row.
    #[must_use]
    pub const fn new(target: (i32, i32)) -> Self {
        Self {
            target,
            side: 0,
            wood_reserve: 4,
            stone_reserve: 0,
            army_wood: 12,
            army_cap: 4,
        }
    }

    /// Field the army and spend the stock of `side` (0 or 1); the town it builds is
    /// that side's own house list.
    #[must_use]
    pub const fn with_side(mut self, side: u8) -> Self {
        self.side = side;
        self
    }

    /// The side this seat works for.
    #[must_use]
    pub const fn side(&self) -> u8 {
        self.side
    }

    /// Never spend wood below this count.
    #[must_use]
    pub const fn with_wood_reserve(mut self, wood: u32) -> Self {
        self.wood_reserve = wood;
        self
    }

    /// Never spend stone below this count.
    #[must_use]
    pub const fn with_stone_reserve(mut self, stone: u32) -> Self {
        self.stone_reserve = stone;
        self
    }

    /// Field a swordsman only while wood is at least this far above the reserve.
    #[must_use]
    pub const fn with_army_wood(mut self, wood: u32) -> Self {
        self.army_wood = wood;
        self
    }

    /// Field at most this many swordsmen.
    #[must_use]
    pub const fn with_army_cap(mut self, units: u32) -> Self {
        self.army_cap = units;
        self
    }

    /// One round of commands, safe to apply in the order returned with
    /// [`Sim::apply_for`] and [`Mayor::side`].
    ///
    /// Apply the whole list, then tick; the next list starts from the state the list leaves.
    #[must_use]
    pub fn decide(&self, sim: &Sim) -> Vec<Command> {
        let mut scratch = sim.clone();
        let mut plan = Vec::new();
        self.lay_roads(&mut scratch, &mut plan);
        for _ in 0..HOUSES_PER_ROUND {
            let Some(kind) = self.next_house(&scratch) else {
                break;
            };
            if !self.pays(&scratch, kind) {
                break;
            }
            let Some((x, y)) = house_site(&scratch, kind) else {
                break;
            };
            let cmd = Command::PlaceHouse { kind, x, y };
            if scratch.apply_for(self.side, cmd).is_err() {
                break;
            }
            plan.push(cmd);
        }
        self.field_swordsman(&mut scratch, &mut plan);
        plan
    }

    /// Extend the road one tile at a time toward the target site. Roads are shared.
    fn lay_roads(&self, scratch: &mut Sim, plan: &mut Vec<Command>) {
        let mut laid = 0;
        for x in 0..=self.target.0 {
            if laid == ROADS_PER_ROUND {
                break;
            }
            let y = self.target.1;
            if scratch.map().is_road(x, y) {
                continue;
            }
            let cmd = Command::PlaceRoad { x, y };
            if scratch.apply_for(self.side, cmd).is_err() {
                break;
            }
            plan.push(cmd);
            laid += 1;
        }
    }

    /// The house the town wants next, driven by what its own stock holds.
    fn next_house(&self, sim: &Sim) -> Option<HouseKind> {
        if count(sim, self.side, HouseKind::Warehouse) == 0 {
            return Some(HouseKind::Warehouse);
        }
        if count(sim, self.side, HouseKind::Woodcutter) == 0 {
            return Some(HouseKind::Woodcutter);
        }
        if count(sim, self.side, HouseKind::Farm) == 0 {
            return Some(HouseKind::Farm);
        }
        // A mill waits for the first grain, a bakery for the first flour.
        if count(sim, self.side, HouseKind::Mill) == 0 && sim.stock_of(self.side, Ware::Grain) > 0 {
            return Some(HouseKind::Mill);
        }
        if count(sim, self.side, HouseKind::Bakery) == 0 && sim.stock_of(self.side, Ware::Flour) > 0
        {
            return Some(HouseKind::Bakery);
        }
        let wood = sim.stock_of(self.side, Ware::Wood);
        let wants = [
            (
                HouseKind::Farm,
                8u32.saturating_sub(sim.stock_of(self.side, Ware::Grain)),
                HOUSES_PER_KIND,
            ),
            (
                HouseKind::Mill,
                6u32.saturating_sub(sim.stock_of(self.side, Ware::Flour)),
                HOUSES_PER_KIND,
            ),
            (
                HouseKind::Bakery,
                4u32.saturating_sub(sim.stock_of(self.side, Ware::Bread)),
                HOUSES_PER_KIND,
            ),
            (
                HouseKind::Woodcutter,
                24u32.saturating_sub(wood),
                WOODCUTTERS,
            ),
        ];
        let mut best: Option<(u32, HouseKind)> = None;
        for (kind, deficit, cap) in wants {
            if deficit == 0 || count(sim, self.side, kind) >= cap {
                continue;
            }
            if best.is_none_or(|(top, _)| deficit > top) {
                best = Some((deficit, kind));
            }
        }
        best.map(|(_, kind)| kind)
    }

    /// True when the house can be paid from the seat's own stock without eating a reserve.
    fn pays(&self, sim: &Sim, kind: HouseKind) -> bool {
        kind.cost().iter().all(|(ware, n)| {
            sim.stock_of(self.side, *ware) >= n.saturating_add(self.reserve(*ware))
        })
    }

    /// The count of one ware the mayor refuses to spend.
    fn reserve(&self, ware: Ware) -> u32 {
        match ware {
            Ware::Wood => self.wood_reserve,
            Ware::Stone => self.stone_reserve,
            Ware::Grain | Ware::Flour | Ware::Bread => 0,
        }
    }

    /// Field one swordsman when wood is above the army threshold and a road tile is free.
    fn field_swordsman(&self, scratch: &mut Sim, plan: &mut Vec<Command>) {
        if !self.wants_swordsman(scratch) {
            return;
        }
        let Some((x, y)) = spawn_site(scratch) else {
            return;
        };
        let cmd = Command::SpawnUnit {
            kind: UnitKind::Swordsman,
            side: self.side,
            x,
            y,
        };
        if scratch.apply_for(self.side, cmd).is_ok() {
            plan.push(cmd);
        }
    }

    /// True while the army has room and the own stock can pay without breaking the reserve.
    fn wants_swordsman(&self, sim: &Sim) -> bool {
        count_units(sim, UnitKind::Swordsman, self.side) < self.army_cap
            && sim.stock_of(self.side, Ware::Wood)
                >= self.army_wood.saturating_add(self.wood_reserve)
    }
}

/// Houses of one kind and side standing in the town.
fn count(sim: &Sim, side: u8, kind: HouseKind) -> u32 {
    let n = sim
        .houses_of(side)
        .filter(|house| house.kind == kind)
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Units of one side and kind standing on the map.
fn count_units(sim: &Sim, kind: UnitKind, side: u8) -> u32 {
    let n = sim
        .units()
        .iter()
        .filter(|u| u.kind == kind && u.side == side)
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// First free footprint that touches a road, in row-major tile order.
fn house_site(sim: &Sim, kind: HouseKind) -> Option<(i32, i32)> {
    let (w, h) = kind.footprint();
    let map = sim.map();
    for y in 0..=map.height().saturating_sub(h) {
        for x in 0..=map.width().saturating_sub(w) {
            if !map.can_place(x, y, w, h) || !map.touches_road(x, y, w, h) {
                continue;
            }
            let taken = sim
                .units()
                .iter()
                .any(|u| u.x >= x && u.x < x + w && u.y >= y && u.y < y + h);
            if !taken {
                return Some((x, y));
            }
        }
    }
    None
}

/// First road tile without a unit, in row-major tile order; units never block a house.
fn spawn_site(sim: &Sim) -> Option<(i32, i32)> {
    let map = sim.map();
    for y in 0..map.height() {
        for x in 0..map.width() {
            if !map.is_road(x, y) || sim.units().iter().any(|u| u.x == x && u.y == y) {
                continue;
            }
            return Some((x, y));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::Mayor;
    use crate::economy::{HouseKind, Ware};
    use crate::sim::{Command, Sim};

    const ROUND: u32 = 25;

    fn play(sim: &mut Sim, mayor: &Mayor, ticks: u32) {
        while sim.ticks() < ticks {
            for cmd in mayor.decide(sim) {
                sim.apply_for(mayor.side(), cmd)
                    .expect("the mayor only issues commands the sim takes");
            }
            for _ in 0..ROUND {
                sim.tick();
            }
        }
    }

    fn first_house(sim: &Sim, mayor: &Mayor) -> Option<HouseKind> {
        mayor.decide(sim).iter().find_map(|cmd| match cmd {
            Command::PlaceHouse { kind, .. } => Some(*kind),
            _ => None,
        })
    }

    fn town() -> Sim {
        Sim::new(24, 18)
    }

    #[test]
    fn the_mayor_builds_the_same_town_twice() {
        let mayor = Mayor::new((20, 12));
        let mut a = town();
        let mut b = town();
        assert_eq!(
            first_house(&a, &mayor),
            Some(HouseKind::Warehouse),
            "the warehouse comes first"
        );
        play(&mut a, &mayor, 3000);
        play(&mut b, &mayor, 3000);
        assert_eq!(a.hash(), b.hash(), "same state, same commands, same hash");
        let warehouses = a
            .houses()
            .iter()
            .filter(|h| h.kind == HouseKind::Warehouse)
            .count();
        assert!(warehouses >= 1, "warehouses {warehouses}");
        assert!(
            a.houses().len() - warehouses >= 3,
            "houses {}",
            a.houses().len()
        );
        assert!(a.stock(Ware::Bread) > 0, "bread {}", a.stock(Ware::Bread));
        assert_eq!(
            a.houses_of(1).count(),
            0,
            "the side-0 seat builds only side 0"
        );
        assert_eq!(
            a.stock_of(1, Ware::Wood),
            40,
            "the other purse is untouched"
        );
    }

    #[test]
    fn the_mayor_keeps_its_reserve() {
        let mayor = Mayor::new((20, 12))
            .with_wood_reserve(20)
            .with_stone_reserve(5);
        let mut sim = Sim::new(24, 18);
        for _ in 0..120 {
            let plan = mayor.decide(&sim);
            assert_eq!(plan, mayor.decide(&sim), "a pure decision");
            for cmd in plan {
                let applied = sim.apply_for(mayor.side(), cmd);
                assert!(applied.is_ok(), "{cmd:?} refused: {:?}", applied.err());
                assert!(
                    sim.stock(Ware::Wood) >= 20,
                    "wood {}",
                    sim.stock(Ware::Wood)
                );
                assert!(
                    sim.stock(Ware::Stone) >= 5,
                    "stone {}",
                    sim.stock(Ware::Stone)
                );
            }
            for _ in 0..ROUND {
                sim.tick();
            }
        }
        assert!(sim.houses().len() >= 4, "houses {}", sim.houses().len());
        assert!(
            sim.stock(Ware::Bread) > 0,
            "bread {}",
            sim.stock(Ware::Bread)
        );
    }

    #[test]
    fn the_mayor_can_seat_side_one() {
        let mayor = Mayor::new((20, 12)).with_side(1);
        let mut sim = town();
        play(&mut sim, &mayor, 900);
        let army = sim
            .units()
            .iter()
            .any(|u| u.kind == crate::units::UnitKind::Swordsman && u.side == 1);
        assert!(army, "side 1 fields swordsmen");
        assert!(
            sim.units().iter().all(|u| u.side == 1),
            "the side-1 seat fields nothing for side 0"
        );
        assert!(sim.houses_of(1).count() > 0, "side 1 builds its own houses");
        assert_eq!(
            sim.houses_of(0).count(),
            0,
            "side 1 builds nothing for side 0"
        );
    }

    #[test]
    fn the_mayor_grows_past_five_houses_on_its_own_stock() {
        let mayor = Mayor::new((20, 12)).with_side(1);
        let mut sim = town();
        play(&mut sim, &mayor, 3000);
        assert!(
            sim.houses_of(1).count() > 5,
            "side 1 placed {} houses",
            sim.houses_of(1).count()
        );
        assert!(
            sim.stock_of(1, Ware::Bread) > 0,
            "side 1 bakes its own bread"
        );
        assert_eq!(sim.houses_of(0).count(), 0, "side 0 built nothing");
        assert_eq!(sim.stock_of(0, Ware::Wood), 40, "side 0 purse untouched");
        assert_eq!(sim.stock_of(0, Ware::Bread), 0, "side 0 ate nothing");
    }
}
