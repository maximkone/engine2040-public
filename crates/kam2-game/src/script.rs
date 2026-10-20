//! The two matches this crate plays: the start village and the pinned fight.
//!
//! Both are plain [`kam2_sim::Sim`] values built from the same command list in
//! the same order, so the window, the tests and a future replay share one
//! fixture. The fight is the hash gate: same commands, same ticks, same number.

use kam2_sim::{Command, HouseKind, Refusal, Sim, UnitKind};

use crate::view::{MAP_H, MAP_W};

/// Row of the single east-west road in the start village.
pub const ROAD_ROW: i32 = 6;
/// Houses of the start village: kind and top-left tile. Each border touches the
/// road, so every house is linked to the warehouse from tick zero.
const START_HOUSES: [(HouseKind, i32, i32); 3] = [
    (HouseKind::Warehouse, 0, 3),
    (HouseKind::Farm, 4, 4),
    (HouseKind::Woodcutter, 13, 4),
];
/// Starting army: side and tile. The two sides stand apart on the open ground
/// south of the road, so the player (or the A key) starts the fight.
const START_UNITS: [(u8, i32, i32); 4] = [(0, 3, 9), (0, 5, 9), (1, 11, 9), (1, 13, 9)];
/// The extra pair of the scripted fight, face to face one tile apart.
const FIGHT_PAIR: [(u8, i32, i32); 2] = [(0, 7, 9), (1, 8, 9)];
/// Ticks the scripted fight plays; more than enough for one swordsman to fall.
pub const FIGHT_TICKS: u32 = 60;

/// The start village: one road, a warehouse, a farm, a woodcutter and four
/// swordsmen. The houses cost side 0 16 wood and 5 stone, and each side pays 6
/// wood for its own two swordsmen, so side 0 still holds 18 wood and 15 stone to
/// build with while side 1 starts with 34 wood and 20 stone.
///
/// # Errors
///
/// Returns the first [`Refusal`] when the fixed layout no longer fits the map.
pub fn match_start() -> Result<Sim, Refusal> {
    let mut sim = Sim::new(MAP_W, MAP_H);
    for x in 0..MAP_W {
        sim.apply(Command::PlaceRoad { x, y: ROAD_ROW })?;
    }
    for (kind, x, y) in START_HOUSES {
        sim.apply(Command::PlaceHouse { kind, x, y })?;
    }
    for (side, x, y) in START_UNITS {
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
    Ok(sim)
}

/// The start village plus two swordsmen face to face, both attack orders, and
/// [`FIGHT_TICKS`] ticks. One swordsman falls; the survivor keeps the field.
///
/// # Errors
///
/// Returns the first [`Refusal`] when the fixed layout or an order no longer fits.
pub fn scripted_fight() -> Result<Sim, Refusal> {
    let mut sim = match_start()?;
    for (side, x, y) in FIGHT_PAIR {
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
    for side in [0u8, 1] {
        if let Some((attacker, target)) = nearest_enemy(&sim, side) {
            sim.apply_for(
                side,
                Command::Attack {
                    id: attacker,
                    target,
                },
            )?;
        }
    }
    for _ in 0..FIGHT_TICKS {
        sim.tick();
    }
    Ok(sim)
}

/// The closest cross-side pair: `(unit of `side`, its nearest enemy)`, chosen by
/// taxicab distance and then by spawn order. This is the A key's order.
#[must_use]
pub fn nearest_enemy(sim: &Sim, side: u8) -> Option<(u32, u32)> {
    let mut best: Option<(i32, u32, u32)> = None;
    for unit in sim.units().iter().filter(|unit| unit.side == side) {
        for enemy in sim.units().iter().filter(|unit| unit.side != side) {
            let distance = (unit.x - enemy.x).abs() + (unit.y - enemy.y).abs();
            if best.is_none_or(|(closest, _, _)| distance < closest) {
                best = Some((distance, unit.id, enemy.id));
            }
        }
    }
    best.map(|(_, attacker, target)| (attacker, target))
}

/// A free tile for a new unit of `side`: the open walkable tile closest to that
/// side's rally point, ties broken by row and then column. `None` when no tile
/// is free.
#[must_use]
pub fn spawn_tile(sim: &Sim, side: u8) -> Option<(i32, i32)> {
    let rally = if side == 0 { (2, 9) } else { (13, 9) };
    let mut best: Option<(i32, i32, i32)> = None;
    for y in 0..MAP_H {
        for x in 0..MAP_W {
            if !sim.map().walkable(x, y)
                || sim.units().iter().any(|unit| unit.x == x && unit.y == y)
            {
                continue;
            }
            let distance = (x - rally.0).abs() + (y - rally.1).abs();
            if best.is_none_or(|(closest, _, _)| distance < closest) {
                best = Some((distance, x, y));
            }
        }
    }
    best.map(|(_, x, y)| (x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kam2_sim::Ware;

    #[test]
    fn the_start_village_places_its_houses_and_army() {
        let sim = match_start().expect("start village");
        assert_eq!(sim.houses().len(), 3);
        assert_eq!(sim.units().len(), 4);
        assert_eq!(sim.stock(Ware::Wood), 18);
        assert_eq!(sim.stock(Ware::Stone), 15);
        for house in sim.houses() {
            assert!(
                house.link.is_some(),
                "{} has no road link",
                house.kind.name()
            );
        }
    }

    #[test]
    fn spawn_tiles_are_free_and_near_the_rally() {
        let sim = match_start().expect("start village");
        assert_eq!(spawn_tile(&sim, 0), Some((2, 9)));
        let west = spawn_tile(&sim, 1).expect("side 1 has room");
        assert!(
            sim.map().walkable(west.0, west.1),
            "{west:?} is not walkable"
        );
        assert!(
            sim.units().iter().all(|unit| (unit.x, unit.y) != west),
            "{west:?} holds a unit"
        );
    }

    #[test]
    fn the_nearest_enemy_is_the_face_to_face_pair() {
        let mut sim = match_start().expect("start village");
        for (side, x, y) in FIGHT_PAIR {
            sim.apply_for(
                side,
                Command::SpawnUnit {
                    kind: UnitKind::Swordsman,
                    side,
                    x,
                    y,
                },
            )
            .expect("the fight pair finds open ground");
        }
        assert_eq!(nearest_enemy(&sim, 0), Some((4, 5)));
        assert_eq!(nearest_enemy(&sim, 1), Some((5, 4)));
        let (attacker, target) = nearest_enemy(&sim, 0).expect("both sides field units");
        sim.apply_for(
            0,
            Command::Attack {
                id: attacker,
                target,
            },
        )
        .expect("attack order");
        let (attacker, target) = nearest_enemy(&sim, 1).expect("both sides field units");
        sim.apply_for(
            1,
            Command::Attack {
                id: attacker,
                target,
            },
        )
        .expect("attack order");
        for _ in 0..FIGHT_TICKS {
            sim.tick();
        }
        assert_eq!(sim.units().len(), 5, "the pair loses one swordsman");
        assert_eq!(sim.unit(4).map(|unit| unit.hp), Some(6));
        assert!(sim.unit(5).is_none(), "id 5 fell in the duel");
    }

    #[test]
    fn the_scripted_fight_ends_on_one_fixed_hash() {
        let fight = scripted_fight().expect("scripted fight");
        assert_eq!(fight.ticks(), FIGHT_TICKS);
        assert_eq!(fight.units().len(), 5, "the four bystanders and the winner");
        assert_eq!(fight.unit(4).map(|unit| unit.hp), Some(6));
        assert!(fight.unit(5).is_none(), "the side-1 swordsman fell");
        assert_eq!(
            fight.hash(),
            0xe065_56a1_4caf_0f7b,
            "pin the state digest of the scripted fight; before the side split it was 0x91c3_22e3_2246_3749"
        );
        let twice = scripted_fight().expect("scripted fight again");
        assert_eq!(fight.hash(), twice.hash(), "same commands, same hash");
    }
}
