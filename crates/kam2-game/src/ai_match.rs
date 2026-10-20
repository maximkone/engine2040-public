//! KAM6: the AI mayor in the side-1 seat, in the same match the player drives.
//!
//! One loop, two callers: the window plays it a tick at a time and the headless
//! `kam2match` binary plays [`RECEIPT_TICKS`] of it (the bin is the receipt).
//! Every [`AI_PERIOD`] ticks the mayor decides on the live [`Sim`] and its
//! commands go in through `apply_for(AI_SIDE, ..)`, exactly like the player's own
//! orders go in for side 0. Each side keeps its own stock, houses and deliveries;
//! roads are shared. Side 1 also defends: an enemy within [`DEFENCE_RANGE`] tiles
//! of its town pulls the nearest swordsman onto that enemy, by attack when in
//! reach and by a walk toward it otherwise. The loop reads no clock, frame or
//! file, so the same start and the same tick count give the same hash.
//!
//! Either side may also hand a unit to the order layer ([`AiMatch::attack_move`],
//! [`AiMatch::approach_to`]); it runs every tick beside the mayor, and its refusals
//! are counted apart from the mayor's own.

use kam2_sim::{Command, HouseKind, Mayor, Refusal, Sim, UnitKind, Ware};

use crate::orders::Orders;
use crate::script;

/// The AI's team. The player holds side 0.
pub const AI_SIDE: u8 = 1;
/// Ticks between two AI decision rounds.
pub const AI_PERIOD: u32 = 20;
/// Enemy tiles this close to the AI town are its business, in taxicab tiles.
pub const DEFENCE_RANGE: i32 = 6;
/// Ticks the headless receipt plays, as the order asks.
pub const RECEIPT_TICKS: u32 = 3000;
/// Width of the KAM6 field, in tiles.
const FIELD_W: i32 = 24;
/// Height of the KAM6 field, in tiles.
const FIELD_H: i32 = 18;
/// Where the AI mayor's road ends: the east end of its single road.
const AI_TARGET: (i32, i32) = (20, 12);
/// The player's two starting swordsmen on the KAM6 field.
const PLAYER_START: [(i32, i32); 2] = [(2, 2), (4, 2)];

/// One KAM6 match: the sim, the side-1 mayor and the AI's own house ids.
pub struct AiMatch {
    sim: Sim,
    mayor: Mayor,
    town: (i32, i32),
    /// True once the town anchor is settled: the AI's first own warehouse.
    town_set: bool,
    ai_houses: Vec<u32>,
    refusals: u32,
    orders: Orders,
}

impl AiMatch {
    /// The KAM6 field: a 24x18 map, two side-0 swordsmen and the AI mayor's town
    /// at [`AI_TARGET`]. The player is idle here; the headless receipt plays it.
    ///
    /// # Errors
    ///
    /// Returns the first [`Refusal`] when the fixed start no longer fits.
    pub fn new() -> Result<Self, Refusal> {
        let mut sim = Sim::new(FIELD_W, FIELD_H);
        for (x, y) in PLAYER_START {
            sim.apply_for(
                0,
                Command::SpawnUnit {
                    kind: UnitKind::Swordsman,
                    side: 0,
                    x,
                    y,
                },
            )?;
        }
        Ok(Self::seated(
            sim,
            Mayor::new(AI_TARGET).with_side(AI_SIDE),
            AI_TARGET,
            false,
        ))
    }

    /// The window's 16x12 start village with the AI in the side-1 seat. The
    /// village and its warehouse belong to side 0; the AI keeps its own stock
    /// and builds its own town beside it, so neither town pays for the other.
    ///
    /// # Errors
    ///
    /// Returns the first [`Refusal`] when the start village no longer fits.
    pub fn village() -> Result<Self, Refusal> {
        let sim = script::match_start()?;
        Ok(Self::seated(
            sim,
            Mayor::new((10, 10)).with_side(AI_SIDE),
            (10, 10),
            false,
        ))
    }

    fn seated(sim: Sim, mayor: Mayor, town: (i32, i32), town_set: bool) -> Self {
        Self {
            sim,
            mayor,
            town,
            town_set,
            ai_houses: Vec::new(),
            refusals: 0,
            orders: Orders::new(),
        }
    }

    /// Read-only match, for the view, the HUD and the player's own orders.
    #[must_use]
    pub const fn sim(&self) -> &Sim {
        &self.sim
    }

    /// Apply one order from the player for side 0; it is never counted as the AI's.
    ///
    /// # Errors
    ///
    /// Returns the sim's [`Refusal`] and changes nothing.
    pub fn order(&mut self, cmd: Command) -> Result<(), Refusal> {
        self.order_for(0, cmd)
    }

    /// Apply one order for `side`; the side's own stock pays and only its units obey.
    ///
    /// # Errors
    ///
    /// Returns the sim's [`Refusal`] and changes nothing.
    pub fn order_for(&mut self, side: u8, cmd: Command) -> Result<(), Refusal> {
        self.sim.apply_for(side, cmd)
    }

    /// Hand `unit` of `side` to the order layer: close on `target`, then strike.
    pub fn attack_move(&mut self, side: u8, unit: u32, target: u32) {
        self.orders.attack_move(side, unit, target);
    }

    /// Hand `unit` of `side` to the order layer: walk to a tile and stop.
    pub fn approach_to(&mut self, side: u8, unit: u32, x: i32, y: i32) {
        self.orders.approach(side, unit, x, y);
    }

    /// Orders the layer still runs.
    #[must_use]
    pub const fn live_orders(&self) -> usize {
        self.orders.len()
    }

    /// Commands the order layer had refused over this match.
    #[must_use]
    pub const fn order_refusals(&self) -> u32 {
        self.orders.refusals()
    }

    /// Play `ticks` ticks, one AI decision round every [`AI_PERIOD`] ticks.
    pub fn advance(&mut self, ticks: u32) {
        let end = self.sim.ticks().saturating_add(ticks);
        while self.sim.ticks() < end {
            if self.sim.ticks().is_multiple_of(AI_PERIOD) {
                self.decide();
            }
            self.orders.tick(&mut self.sim);
            self.sim.tick();
        }
    }

    /// One round: the mayor's whole plan, then the defence order.
    fn decide(&mut self) {
        for cmd in self.mayor.decide(&self.sim) {
            match self.sim.apply_for(AI_SIDE, cmd) {
                Ok(()) => {
                    if let Command::PlaceHouse { kind, .. } = cmd {
                        self.record_house(kind);
                    }
                }
                Err(_) => self.refusals = self.refusals.saturating_add(1),
            }
        }
        self.defend();
    }

    /// Track a house the AI just placed; the first own warehouse becomes its town.
    fn record_house(&mut self, kind: HouseKind) {
        let Some(house) = self.sim.houses_of(AI_SIDE).last() else {
            return;
        };
        self.ai_houses.push(house.id);
        if !self.town_set && kind == HouseKind::Warehouse {
            let (w, h) = kind.footprint();
            self.town = (house.x + w / 2, house.y + h / 2);
            self.town_set = true;
        }
    }

    /// Send the nearest swordsman onto an enemy that stands near the town.
    fn defend(&mut self) {
        let Some(enemy) = self.enemy_near_town() else {
            return;
        };
        let Some(cell) = self.sim.unit(enemy).map(|u| (u.x, u.y)) else {
            return;
        };
        let Some((id, x, y, target)) = self.nearest_defender(cell) else {
            return;
        };
        if target == Some(enemy) {
            return;
        }
        let reach = i32::try_from(UnitKind::Swordsman.range()).unwrap_or(1);
        if (x - cell.0).abs() + (y - cell.1).abs() <= reach {
            self.issue(Command::Attack { id, target: enemy });
        } else if let Some((tx, ty)) = self.approach((x, y), cell) {
            self.issue(Command::MoveUnit { id, x: tx, y: ty });
        }
    }

    /// The closest enemy of the AI inside the town's defence ring, ties by id.
    fn enemy_near_town(&self) -> Option<u32> {
        let mut best: Option<(i32, u32)> = None;
        for unit in self.sim.units().iter().filter(|u| u.side != AI_SIDE) {
            let distance = (unit.x - self.town.0).abs() + (unit.y - self.town.1).abs();
            if distance > DEFENCE_RANGE {
                continue;
            }
            if best.is_none_or(|(closest, _)| distance < closest) {
                best = Some((distance, unit.id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// The side-1 swordsman closest to `cell`, with its tile and current target.
    fn nearest_defender(&self, cell: (i32, i32)) -> Option<(u32, i32, i32, Option<u32>)> {
        let mut best: Option<(i32, u32, i32, i32, Option<u32>)> = None;
        for unit in self
            .sim
            .units()
            .iter()
            .filter(|u| u.side == AI_SIDE && u.kind == UnitKind::Swordsman)
        {
            let distance = (unit.x - cell.0).abs() + (unit.y - cell.1).abs();
            if best.is_none_or(|(closest, _, _, _, _)| distance < closest) {
                best = Some((distance, unit.id, unit.x, unit.y, unit.target));
            }
        }
        best.map(|(_, id, x, y, target)| (id, x, y, target))
    }

    /// A free walkable tile beside `cell`, nearest to the defender, ties by row.
    fn approach(&self, from: (i32, i32), cell: (i32, i32)) -> Option<(i32, i32)> {
        let mut best: Option<(i32, i32, i32)> = None;
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let (x, y) = (cell.0 + dx, cell.1 + dy);
            if !self.sim.map().walkable(x, y)
                || self.sim.units().iter().any(|u| u.x == x && u.y == y)
            {
                continue;
            }
            let distance = (x - from.0).abs() + (y - from.1).abs();
            if best.is_none_or(|(closest, _, _)| distance < closest) {
                best = Some((distance, x, y));
            }
        }
        best.map(|(_, x, y)| (x, y))
    }

    /// Apply an AI order; a refusal is counted so the proofs can demand zero.
    fn issue(&mut self, cmd: Command) {
        if self.sim.apply_for(AI_SIDE, cmd).is_err() {
            self.refusals = self.refusals.saturating_add(1);
        }
    }

    /// AI houses still standing.
    #[must_use]
    pub fn ai_houses(&self) -> usize {
        self.sim
            .houses_of(AI_SIDE)
            .filter(|house| self.ai_houses.contains(&house.id))
            .count()
    }

    /// AI orders the sim refused over this match.
    #[must_use]
    pub const fn refusals(&self) -> u32 {
        self.refusals
    }

    /// The one receipt line of a headless match.
    #[must_use]
    pub fn receipt(&self) -> String {
        format!(
            "KAM2 AI OK tick={} houses={} units={} side1_houses={} hash={:016x}",
            self.sim.ticks(),
            self.sim.houses().len(),
            self.sim.units().len(),
            self.ai_houses(),
            self.sim.hash()
        )
    }

    /// The two-purse receipt: each side's own house count and bread, one hash.
    #[must_use]
    pub fn sides_receipt(&self) -> String {
        format!(
            "KAM2 SIDES OK tick={} side0_houses={} side1_houses={} side0_bread={} side1_bread={} hash={:016x}",
            self.sim.ticks(),
            self.sim.houses_of(0).count(),
            self.sim.houses_of(AI_SIDE).count(),
            self.sim.stock_of(0, Ware::Bread),
            self.sim.stock_of(AI_SIDE, Ware::Bread),
            self.sim.hash()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{AiMatch, AI_SIDE, RECEIPT_TICKS};
    use kam2_sim::{Command, UnitKind, Ware};

    fn played() -> AiMatch {
        let mut game = AiMatch::new().expect("the KAM6 field starts");
        game.advance(RECEIPT_TICKS);
        game
    }

    #[test]
    fn the_ai_town_grows_past_five_houses() {
        let game = played();
        assert!(
            game.ai_houses() > 5,
            "the AI placed {} houses",
            game.ai_houses()
        );
        assert!(
            game.sim().houses().len() > 5,
            "the field holds {} houses",
            game.sim().houses().len()
        );
        assert_eq!(
            game.sim().houses_of(AI_SIDE).count(),
            game.ai_houses(),
            "every AI house sits in the side-1 list"
        );
        assert_eq!(
            game.sim().houses_of(0).count(),
            0,
            "the side-1 seat built nothing for side 0"
        );
        assert_eq!(
            game.sim().stock_of(0, Ware::Wood),
            34,
            "two side-0 swordsmen cost six wood; the AI spent none of it"
        );
    }

    #[test]
    fn the_ai_fields_swordsmen() {
        let game = played();
        let army = game
            .sim()
            .units()
            .iter()
            .filter(|unit| unit.side == AI_SIDE && unit.kind == UnitKind::Swordsman)
            .count();
        assert!(army > 0, "the side-1 seat fields {army} swordsmen");
    }

    #[test]
    fn the_village_ai_builds_its_own_town_beside_the_player() {
        let mut game = AiMatch::village().expect("the start village places");
        game.advance(RECEIPT_TICKS);
        assert_eq!(
            game.sim().houses_of(0).count(),
            3,
            "the player's three houses stay the player's"
        );
        assert!(
            game.sim().houses_of(AI_SIDE).count() > 1,
            "side 1 grows its own house list"
        );
    }

    #[test]
    fn the_match_hash_is_stable_twice() {
        let first = played();
        let second = played();
        assert_eq!(
            first.sim().hash(),
            second.sim().hash(),
            "same start, same ticks, same hash"
        );
    }

    #[test]
    fn no_ai_command_is_refused() {
        let game = played();
        assert_eq!(
            game.refusals(),
            0,
            "the sim refused {} AI orders",
            game.refusals()
        );
    }

    #[test]
    fn an_enemy_near_the_town_pulls_the_nearest_defender() {
        let mut game = AiMatch::new().expect("the KAM6 field starts");
        game.advance(200);
        let player = game
            .sim()
            .units()
            .iter()
            .find(|unit| unit.side == 0)
            .map(|unit| unit.id)
            .expect("the player still holds the field");
        game.order(Command::MoveUnit {
            id: player,
            x: 2,
            y: 6,
        })
        .expect("the enemy walks toward the town");
        game.advance(40);
        let engaged = game.sim().units().iter().any(|unit| {
            unit.side == AI_SIDE
                && unit.kind == UnitKind::Swordsman
                && (unit.target == Some(player) || !unit.path.is_empty())
        });
        assert!(
            engaged,
            "a side-1 swordsman takes the enemy at the town gate"
        );
    }

    #[test]
    fn the_order_layer_closes_on_its_target_inside_the_match() {
        let mut game = AiMatch::new().expect("the KAM6 field starts");
        game.order_for(
            AI_SIDE,
            Command::SpawnUnit {
                kind: UnitKind::Swordsman,
                side: AI_SIDE,
                x: 6,
                y: 2,
            },
        )
        .expect("a side-1 swordsman starts beside the player");
        let player = game
            .sim()
            .units()
            .iter()
            .find(|unit| unit.side == 0)
            .map(|unit| unit.id)
            .expect("side 0 holds the field");
        let enemy = game
            .sim()
            .units()
            .iter()
            .find(|unit| unit.side == AI_SIDE && (unit.x, unit.y) == (6, 2))
            .map(|unit| unit.id)
            .expect("the new swordsman stands there");
        let gap = |game: &AiMatch| {
            let from = game.sim().unit(player)?;
            let to = game.sim().unit(enemy)?;
            Some((from.x - to.x).abs() + (from.y - to.y).abs())
        };
        let before = gap(&game).expect("both units live");
        game.attack_move(AI_SIDE, enemy, player);
        game.advance(40);
        let after = gap(&game);
        assert!(
            after.is_none_or(|now| now < before),
            "the gap {before} did not fall (now {after:?})"
        );
        assert_eq!(
            game.order_refusals(),
            0,
            "the layer had {} commands refused",
            game.order_refusals()
        );
        assert!(game.live_orders() <= 1, "one order drives one unit");
    }
}
