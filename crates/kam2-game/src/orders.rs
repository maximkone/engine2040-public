//! CMB1: the order layer above the sim - close the distance, then strike.
//!
//! `Attack` holds position, so a swordsman ordered onto a distant enemy stands still.
//! [`Orders`] keeps that intent here, above the sim, and each tick issues only public
//! [`Command`]s: `MoveUnit` toward a free tile beside the target while the target is out
//! of reach, `Attack` once it is in reach, and a fresh `MoveUnit` when the target steps
//! onto another tile or the walk is blocked. Both sides use the same layer; every
//! refusal is counted, never hidden, and the proofs demand zero.
//!
//! No floats, no clock: the layer reads the live [`Sim`] and writes commands into it.

use kam2_sim::{Command, Refusal, Sim, Unit, UnitKind};

/// Neighbour order for every tie: west, east, north, south.
const NEIGHBOURS: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];

/// What one unit was told to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderKind {
    /// Close on `target`, then strike it until it or the unit falls.
    AttackMove {
        /// Unit to close on and strike.
        target: u32,
    },
    /// Walk to a tile and stop there.
    Approach {
        /// Destination tile.
        x: i32,
        /// Destination tile.
        y: i32,
    },
}

/// One order: the side that gave it, the unit that obeys, and the goal.
#[derive(Clone, Copy, Debug)]
pub struct Order {
    /// Side that owns the unit; only that side's commands are issued.
    pub side: u8,
    /// Unit that obeys.
    pub unit: u32,
    /// What it must do.
    pub kind: OrderKind,
    /// Target tile the running path was planned for; a new tile re-paths.
    last_target: Option<(i32, i32)>,
    /// Tile the unit stood on at the last order tick.
    last_tile: Option<(i32, i32)>,
    /// Order ticks spent with a path and no step taken.
    stall: u32,
}

/// The running orders of one match, driven once per tick.
#[derive(Debug, Default)]
pub struct Orders {
    orders: Vec<Order>,
    refusals: u32,
    moves: u32,
    attacks: u32,
}

impl Orders {
    /// No orders yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Order `unit` of `side` to close on `target` and strike it in reach.
    pub fn attack_move(&mut self, side: u8, unit: u32, target: u32) {
        self.push(Order {
            side,
            unit,
            kind: OrderKind::AttackMove { target },
            last_target: None,
            last_tile: None,
            stall: 0,
        });
    }

    /// Order `unit` of `side` to walk to a tile and stop there.
    pub fn approach(&mut self, side: u8, unit: u32, x: i32, y: i32) {
        self.push(Order {
            side,
            unit,
            kind: OrderKind::Approach { x, y },
            last_target: None,
            last_tile: None,
            stall: 0,
        });
    }

    /// One order per unit: a new order for the same unit replaces the old one.
    fn push(&mut self, order: Order) {
        self.orders
            .retain(|old| !(old.unit == order.unit && old.side == order.side));
        self.orders.push(order);
    }

    /// One round of orders: drop finished ones, then issue one command each.
    pub fn tick(&mut self, sim: &mut Sim) {
        self.retire(sim);
        let mut pending = std::mem::take(&mut self.orders);
        for order in &mut pending {
            self.drive(order, sim);
        }
        retain_alive(&mut pending, sim);
        self.orders = pending;
    }

    /// Drop orders whose unit or target is gone, or whose walk to a tile is done.
    pub fn retire(&mut self, sim: &Sim) {
        retain_alive(&mut self.orders, sim);
    }

    /// How many orders still run.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.orders.len()
    }

    /// True when no order runs: every unit arrived, died, or lost its target.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    /// Commands the sim refused over this match.
    #[must_use]
    pub const fn refusals(&self) -> u32 {
        self.refusals
    }

    /// `MoveUnit` commands issued, a proof hook for the tests.
    #[must_use]
    pub const fn move_orders(&self) -> u32 {
        self.moves
    }

    /// `Attack` commands issued, a proof hook for the tests.
    #[must_use]
    pub const fn attack_orders(&self) -> u32 {
        self.attacks
    }

    /// Issue the one command this order wants now, if any.
    fn drive(&mut self, order: &mut Order, sim: &mut Sim) {
        let Some(unit) = sim.unit(order.unit) else {
            return;
        };
        let tile = (unit.x, unit.y);
        if order.last_tile == Some(tile) {
            if unit.path.is_empty() {
                order.stall = 0;
            } else {
                order.stall = order.stall.saturating_add(1);
            }
        } else {
            order.last_tile = Some(tile);
            order.stall = 0;
        }
        let forced = order.stall > unit.kind.move_ticks();
        let Some((command, seen)) = plan(order, sim, forced) else {
            return;
        };
        match command {
            Command::MoveUnit { .. } => {
                self.moves = self.moves.saturating_add(1);
                order.stall = 0;
            }
            Command::Attack { .. } => self.attacks = self.attacks.saturating_add(1),
            Command::PlaceRoad { .. } | Command::PlaceHouse { .. } | Command::SpawnUnit { .. } => {
                return;
            }
        }
        if sim.apply_for(order.side, command).is_err() {
            self.refusals = self.refusals.saturating_add(1);
        }
        order.last_target = seen;
    }
}

/// Drop every order that has nothing left to do.
fn retain_alive(orders: &mut Vec<Order>, sim: &Sim) {
    orders.retain(|order| alive(order, sim));
}

/// An order lives while its unit lives and its goal still exists.
fn alive(order: &Order, sim: &Sim) -> bool {
    let Some(unit) = sim.unit(order.unit) else {
        return false;
    };
    if unit.side != order.side {
        return false;
    }
    match order.kind {
        OrderKind::AttackMove { target } => target != order.unit && sim.unit(target).is_some(),
        OrderKind::Approach { x, y } => (unit.x, unit.y) != (x, y),
    }
}

/// The command the order wants this tick, and the target tile its path is for.
fn plan(order: &Order, sim: &Sim, forced: bool) -> Option<(Command, Option<(i32, i32)>)> {
    let unit = sim.unit(order.unit)?;
    match order.kind {
        OrderKind::AttackMove { target } => attack_move(order, unit, sim, target, forced),
        OrderKind::Approach { x, y } => {
            approach(unit, sim, (x, y), forced).map(|cmd| (cmd, order.last_target))
        }
    }
}

/// Walk toward the target while out of reach; strike once in reach.
fn attack_move(
    order: &Order,
    unit: &Unit,
    sim: &Sim,
    target_id: u32,
    forced: bool,
) -> Option<(Command, Option<(i32, i32)>)> {
    let target = sim.unit(target_id)?;
    let reach = i32::try_from(unit.kind.range()).unwrap_or(1);
    let distance = (unit.x - target.x).abs() + (unit.y - target.y).abs();
    if distance <= reach {
        if unit.target == Some(target_id) {
            return None;
        }
        return Some((
            Command::Attack {
                id: unit.id,
                target: target_id,
            },
            order.last_target,
        ));
    }
    let cell = (target.x, target.y);
    if !forced && !unit.path.is_empty() && order.last_target == Some(cell) {
        return None;
    }
    let goal = step_tile(sim, (unit.x, unit.y), cell, unit.id)?;
    Some((
        Command::MoveUnit {
            id: unit.id,
            x: goal.0,
            y: goal.1,
        },
        Some(cell),
    ))
}

/// Walk to a tile; a blocked walk re-paths only after the stall counter calls it.
fn approach(unit: &Unit, sim: &Sim, goal: (i32, i32), forced: bool) -> Option<Command> {
    if (unit.x, unit.y) == goal {
        return None;
    }
    if !forced && !unit.path.is_empty() {
        return None;
    }
    if !reachable(sim, (unit.x, unit.y), goal, unit.id) {
        return None;
    }
    Some(Command::MoveUnit {
        id: unit.id,
        x: goal.0,
        y: goal.1,
    })
}

/// The free walkable tile beside `cell` nearest to `from`, ties by [`NEIGHBOURS`] order.
fn step_tile(sim: &Sim, from: (i32, i32), cell: (i32, i32), mover: u32) -> Option<(i32, i32)> {
    let mut candidates: Vec<(i32, usize, (i32, i32))> = NEIGHBOURS
        .into_iter()
        .enumerate()
        .map(|(index, (dx, dy))| {
            let tile = (cell.0 + dx, cell.1 + dy);
            let distance = (tile.0 - from.0).abs() + (tile.1 - from.1).abs();
            (distance, index, tile)
        })
        .collect();
    candidates.sort_unstable();
    candidates
        .into_iter()
        .map(|(_, _, tile)| tile)
        .find(|tile| {
            sim.map().walkable(tile.0, tile.1)
                && !blocked(sim, *tile, mover)
                && reachable(sim, from, *tile, mover)
        })
}

/// True when another unit stands on `tile`.
fn blocked(sim: &Sim, tile: (i32, i32), mover: u32) -> bool {
    sim.units()
        .iter()
        .any(|unit| unit.id != mover && (unit.x, unit.y) == tile)
}

/// True when an open walk joins `from` and `to`, with the same rules the sim paths by.
fn reachable(sim: &Sim, from: (i32, i32), to: (i32, i32), mover: u32) -> bool {
    if from == to {
        return true;
    }
    if !sim.map().walkable(to.0, to.1) || blocked(sim, to, mover) {
        return false;
    }
    let mut seen = vec![from];
    let mut queue = std::collections::VecDeque::from([from]);
    while let Some((x, y)) = queue.pop_front() {
        for (dx, dy) in NEIGHBOURS {
            let tile = (x + dx, y + dy);
            if tile == to {
                return true;
            }
            if !sim.map().walkable(tile.0, tile.1)
                || blocked(sim, tile, mover)
                || seen.contains(&tile)
            {
                continue;
            }
            seen.push(tile);
            queue.push_back(tile);
        }
    }
    false
}

/// Tiles between the two swordsmen of the order skirmish, on open ground.
pub const SKIRMISH_GAP: i32 = 8;
/// Ticks the order skirmish may play before it counts as a stall.
pub const SKIRMISH_TICKS: u32 = 400;
/// Start tiles of the skirmish: side, x, y. The units take ids 0 and 1.
const SKIRMISH_START: [(u8, i32, i32); 2] = [(0, 1, 3), (1, 9, 3)];

/// Two swordsmen eight tiles apart on open ground, each attack-moving the other.
///
/// # Errors
///
/// Returns the first [`Refusal`] when the fixed start no longer fits.
pub fn skirmish() -> Result<(Sim, Orders), Refusal> {
    let mut sim = Sim::new(12, 6);
    for (side, x, y) in SKIRMISH_START {
        sim.apply_for(
            side,
            Command::SpawnUnit {
                kind: UnitKind::Swordsman,
                side,
                x,
                y,
            },
        )?;
    }
    let mut orders = Orders::new();
    orders.attack_move(0, 0, 1);
    orders.attack_move(1, 1, 0);
    Ok((sim, orders))
}

#[cfg(test)]
mod tests {
    use super::{skirmish, Orders, SKIRMISH_TICKS};
    use kam2_sim::{Command, Sim, UnitKind};

    /// Taxicab tiles between the two skirmish swordsmen, while both live.
    fn gap(sim: &Sim) -> Option<i32> {
        let first = sim.unit(0)?;
        let second = sim.unit(1)?;
        Some((first.x - second.x).abs() + (first.y - second.y).abs())
    }

    /// One full run of the skirmish; stops on the tick the last order ends.
    fn played() -> (Sim, Orders) {
        let (mut sim, mut orders) = skirmish().expect("the skirmish starts");
        for _ in 0..SKIRMISH_TICKS {
            orders.tick(&mut sim);
            if orders.is_empty() {
                break;
            }
            sim.tick();
            orders.retire(&sim);
            if orders.is_empty() {
                break;
            }
        }
        (sim, orders)
    }

    #[test]
    fn the_gap_falls_while_both_close_in() {
        let (mut sim, mut orders) = skirmish().expect("the skirmish starts");
        let start = gap(&sim).expect("both units live");
        let mut previous = start;
        let mut steps = 0;
        while gap(&sim).expect("both units live") > 1 {
            orders.tick(&mut sim);
            sim.tick();
            let now = gap(&sim).expect("both units live");
            assert!(now <= previous, "the gap grew from {previous} to {now}");
            if now < previous {
                steps += 1;
            }
            previous = now;
        }
        assert!(steps >= 3, "the units took {steps} closing steps");
        assert!(previous < start, "the gap fell from {start} to {previous}");
        assert_eq!(orders.attack_orders(), 0, "no strike before contact");
        assert!(
            sim.unit(0).is_some_and(|unit| unit.x != 1)
                && sim.unit(1).is_some_and(|unit| unit.x != 9),
            "both sides walked toward each other"
        );
    }

    #[test]
    fn no_attack_is_ordered_out_of_range() {
        let (mut sim, mut orders) = skirmish().expect("the skirmish starts");
        while gap(&sim).expect("both units live") > 1 {
            orders.tick(&mut sim);
            assert_eq!(
                orders.attack_orders(),
                0,
                "an Attack went out while the target stood out of reach"
            );
            sim.tick();
        }
        orders.tick(&mut sim);
        assert!(
            orders.attack_orders() > 0,
            "in reach the order strikes at once"
        );
        assert_eq!(orders.refusals(), 0);
    }

    #[test]
    fn the_order_stops_when_the_target_dies() {
        let (sim, orders) = played();
        assert!(orders.is_empty(), "the order layer still holds work");
        assert_eq!(sim.units().len(), 1, "one swordsman fell");
        assert!(
            sim.units()[0].hp > 0,
            "the survivor stands with {} hp",
            sim.units()[0].hp
        );
    }

    #[test]
    fn no_order_is_refused_across_the_fight() {
        let (_, orders) = played();
        assert_eq!(
            orders.refusals(),
            0,
            "the sim refused {} order commands",
            orders.refusals()
        );
        assert!(orders.move_orders() > 0, "the layer walked the units in");
        assert!(orders.attack_orders() > 0, "the layer struck in reach");
    }

    #[test]
    fn the_skirmish_hash_is_stable_twice() {
        let first = played();
        let second = played();
        assert_eq!(
            first.0.hash(),
            second.0.hash(),
            "same orders, same ticks, same hash"
        );
        assert_eq!(first.1.refusals(), 0);
    }

    #[test]
    fn an_approach_walks_to_its_tile_and_stops() {
        let mut sim = Sim::new(12, 6);
        sim.apply_for(
            0,
            Command::SpawnUnit {
                kind: UnitKind::Swordsman,
                side: 0,
                x: 1,
                y: 3,
            },
        )
        .expect("the walker spawns");
        let mut orders = Orders::new();
        orders.approach(0, 0, 5, 3);
        for _ in 0..40 {
            orders.tick(&mut sim);
            if orders.is_empty() {
                break;
            }
            sim.tick();
            orders.retire(&sim);
        }
        assert!(orders.is_empty(), "the walk ended");
        assert_eq!(sim.unit(0).map(|unit| (unit.x, unit.y)), Some((5, 3)));
        assert_eq!(sim.ticks(), 16, "four tiles at four ticks each");
        assert_eq!(orders.refusals(), 0);
    }

    #[test]
    fn a_target_that_steps_away_is_followed() {
        let mut sim = Sim::new(14, 8);
        for (side, x, y) in [(0u8, 1, 4), (1, 5, 4)] {
            sim.apply_for(
                side,
                Command::SpawnUnit {
                    kind: UnitKind::Swordsman,
                    side,
                    x,
                    y,
                },
            )
            .expect("the pair spawns");
        }
        let mut orders = Orders::new();
        orders.attack_move(0, 0, 1);
        for _ in 0..4 {
            orders.tick(&mut sim);
            sim.tick();
        }
        let old_goal = sim.unit(0).and_then(|unit| unit.path.back().copied());
        sim.apply_for(1, Command::MoveUnit { id: 1, x: 5, y: 2 })
            .expect("the target steps north");
        for _ in 0..4 {
            orders.tick(&mut sim);
            sim.tick();
        }
        orders.tick(&mut sim);
        let target = sim.unit(1).expect("the target lives");
        let new_goal = sim
            .unit(0)
            .and_then(|unit| unit.path.back().copied())
            .expect("the follower holds a path");
        let beside = (new_goal.0 - target.x).abs() + (new_goal.1 - target.y).abs();
        assert!(
            beside <= 1,
            "the new path ends {new_goal:?}, not beside the target"
        );
        assert_ne!(
            old_goal,
            Some(new_goal),
            "the walk to the old tile was dropped"
        );
        assert_eq!(orders.refusals(), 0);
    }

    #[test]
    fn an_order_for_the_other_side_is_dropped_without_a_command() {
        let mut sim = Sim::new(8, 8);
        for (side, x, y) in [(0u8, 2, 2), (1, 5, 2)] {
            sim.apply_for(
                side,
                Command::SpawnUnit {
                    kind: UnitKind::Swordsman,
                    side,
                    x,
                    y,
                },
            )
            .expect("the pair spawns");
        }
        let mut orders = Orders::new();
        orders.attack_move(1, 0, 1);
        orders.attack_move(0, 0, 0);
        orders.tick(&mut sim);
        assert!(orders.is_empty(), "a foreign or self order is dropped");
        assert_eq!(orders.refusals(), 0);
        assert_eq!(sim.unit(0).and_then(|unit| unit.target), None);
    }
}
