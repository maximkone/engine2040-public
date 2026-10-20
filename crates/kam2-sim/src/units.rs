//! Units and combat: the stat table, walking over open ground, melee and arrows.
//!
//! Invariants: `Sim::tick` is the only clock; a path is a plain tile list from a
//! breadth-first search; every unit field is in the hash, so the same commands and the
//! same ticks repeat the same state.
//!
//! A tile holds one body. The movement phase samples the tile every living unit holds,
//! then walks the units in spawn order: a unit may enter its next tile only when that
//! tile was free in the sample and no earlier unit has entered it this tick. A blocked
//! unit stays on its tile and tries again next tick, so a tile vacated this tick is free
//! only next tick: two units never share a tile, never swap in one tick and never pass
//! through each other. A dead unit releases its tile in the same tick it falls, and a
//! spawn on a held tile is refused with `Refusal::Occupied`.

use std::collections::VecDeque;

use crate::economy::Ware;
use crate::map::Map;

/// How many unit kinds the stat table knows.
pub const UNIT_KIND_COUNT: usize = 3;

/// What a unit is and what it can do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnitKind {
    /// Melee line infantry: sturdy, slow to forget a grudge.
    Swordsman,
    /// Ranged unit; the damage lands after the arrow's flight.
    Archer,
    /// Cheap hand for the economy; almost harmless in a fight.
    Worker,
}

impl UnitKind {
    /// Every kind, in index order.
    pub const ALL: [UnitKind; UNIT_KIND_COUNT] =
        [UnitKind::Swordsman, UnitKind::Archer, UnitKind::Worker];

    /// Position in a stable list; used by the hash and by manifests.
    #[must_use]
    pub const fn index(self) -> u8 {
        match self {
            UnitKind::Swordsman => 0,
            UnitKind::Archer => 1,
            UnitKind::Worker => 2,
        }
    }

    /// Short name for logs and manifests.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            UnitKind::Swordsman => "swordsman",
            UnitKind::Archer => "archer",
            UnitKind::Worker => "worker",
        }
    }

    /// Full health.
    #[must_use]
    pub const fn hp(self) -> u32 {
        match self {
            UnitKind::Swordsman => 30,
            UnitKind::Archer => 18,
            UnitKind::Worker => 12,
        }
    }

    /// Raw damage of one hit, before the target's defence.
    #[must_use]
    pub const fn attack(self) -> u32 {
        match self {
            UnitKind::Swordsman => 8,
            UnitKind::Archer => 6,
            UnitKind::Worker => 1,
        }
    }

    /// Damage removed from every incoming hit.
    #[must_use]
    pub const fn defence(self) -> u32 {
        match self {
            UnitKind::Swordsman => 2,
            UnitKind::Archer => 1,
            UnitKind::Worker => 0,
        }
    }

    /// How far the unit sees, in tiles. Kept for the AI; combat reads only `range`.
    #[must_use]
    pub const fn sight(self) -> u32 {
        match self {
            UnitKind::Swordsman => 6,
            UnitKind::Archer => 9,
            UnitKind::Worker => 5,
        }
    }

    /// Ticks to cross one tile.
    #[must_use]
    pub const fn move_ticks(self) -> u32 {
        match self {
            UnitKind::Swordsman => 4,
            UnitKind::Archer => 5,
            UnitKind::Worker => 6,
        }
    }

    /// Ticks between two strikes.
    #[must_use]
    pub const fn attack_period(self) -> u32 {
        match self {
            UnitKind::Swordsman => 10,
            UnitKind::Archer => 14,
            UnitKind::Worker => 20,
        }
    }

    /// Taxicab reach in tiles.
    #[must_use]
    pub const fn range(self) -> u32 {
        match self {
            UnitKind::Swordsman => 1,
            UnitKind::Archer => 3,
            UnitKind::Worker => 1,
        }
    }

    /// Ticks an arrow stays in the air; zero means the hit lands at once.
    #[must_use]
    pub const fn flight(self) -> u32 {
        match self {
            UnitKind::Swordsman => 0,
            UnitKind::Archer => 4,
            UnitKind::Worker => 0,
        }
    }

    /// Wares spent to field one unit.
    #[must_use]
    pub const fn cost(self) -> &'static [(Ware, u32)] {
        match self {
            UnitKind::Swordsman => &[(Ware::Wood, 3)],
            UnitKind::Archer => &[(Ware::Wood, 2), (Ware::Stone, 2)],
            UnitKind::Worker => &[(Ware::Wood, 1)],
        }
    }
}

/// One unit on the map.
#[derive(Clone, Debug)]
pub struct Unit {
    /// Stable id, handed out in spawn order.
    pub id: u32,
    /// What it is.
    pub kind: UnitKind,
    /// Team: 0 or 1.
    pub side: u8,
    /// Tile.
    pub x: i32,
    /// Tile.
    pub y: i32,
    /// Health left; at zero the unit is removed.
    pub hp: u32,
    /// Ticks until the next strike.
    pub reload: u32,
    /// Unit this one strikes whenever it is in reach.
    pub target: Option<u32>,
    /// Tiles still to walk, next tile first.
    pub path: VecDeque<(i32, i32)>,
    /// Ticks spent on the next tile, from zero to `move_ticks`.
    pub progress: u32,
}

/// An arrow in flight; damage is fixed when it leaves the bow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    /// Shooter's id.
    pub from: u32,
    /// Unit that will take the damage.
    pub target: u32,
    /// Tick the arrow lands.
    pub due: u32,
    /// Health removed after the target's defence.
    pub damage: u32,
}

/// The unit standing on a tile, if any.
#[must_use]
pub(crate) fn unit_at(units: &[Unit], x: i32, y: i32) -> Option<u32> {
    units.iter().find(|u| u.x == x && u.y == y).map(|u| u.id)
}

/// Shortest open path from `from` to `to`, start excluded and goal included.
///
/// Tiles holding a house, water, rock or another unit are not open. Neighbours are
/// tried in one fixed order so the same map always gives the same path.
#[must_use]
pub(crate) fn find_path(
    map: &Map,
    units: &[Unit],
    from: (i32, i32),
    to: (i32, i32),
    mover: u32,
) -> Option<VecDeque<(i32, i32)>> {
    let start = map.index(from.0, from.1)?;
    let goal = map.index(to.0, to.1)?;
    if start == goal {
        return Some(VecDeque::new());
    }
    let taken: Vec<(i32, i32)> = units
        .iter()
        .filter(|u| u.id != mover)
        .map(|u| (u.x, u.y))
        .collect();
    let size = usize::try_from(map.width().max(0) * map.height().max(0)).ok()?;
    if size == 0 {
        return None;
    }
    let mut seen = vec![false; size];
    let mut parent = vec![usize::MAX; size];
    seen[start] = true;
    let mut queue = VecDeque::new();
    queue.push_back(start);
    while let Some(i) = queue.pop_front() {
        if i == goal {
            break;
        }
        let Some((x, y)) = map.coords(i) else {
            break;
        };
        for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
            let Some(n) = map.index(nx, ny) else {
                continue;
            };
            if seen[n] || !map.walkable(nx, ny) || taken.contains(&(nx, ny)) {
                continue;
            }
            seen[n] = true;
            parent[n] = i;
            queue.push_back(n);
        }
    }
    if !seen[goal] {
        return None;
    }
    let mut path = VecDeque::new();
    let mut at = goal;
    while at != start {
        let step = parent[at];
        if step == usize::MAX {
            return None;
        }
        let (x, y) = map.coords(at)?;
        path.push_front((x, y));
        at = step;
    }
    Some(path)
}

/// One tick of the army: arrows land, units walk, units strike.
///
/// Returns how many tile entries had to wait for a body; the match adds it to its
/// blocked-move counter.
pub(crate) fn step(tick: u32, map: &Map, units: &mut Vec<Unit>, shots: &mut Vec<Shot>) -> u32 {
    land_shots(tick, units, shots);
    let blocked = advance_movement(map, units);
    strike(tick, units, shots);
    blocked
}

fn land_shots(tick: u32, units: &mut Vec<Unit>, shots: &mut Vec<Shot>) {
    let in_flight = std::mem::take(shots);
    for shot in in_flight {
        if shot.due <= tick {
            if let Some(target) = units.iter_mut().find(|u| u.id == shot.target) {
                target.hp = target.hp.saturating_sub(shot.damage);
            }
        } else {
            shots.push(shot);
        }
    }
    units.retain(|u| u.hp > 0);
}

/// One movement phase: every unit at its arrival tick reserves the next tile, or waits.
///
/// `before` is the tile sample of this tick; `claimed` holds what earlier units entered.
/// A unit that cannot enter keeps itself one tick short of arrival, so it retries on the
/// next tick instead of spending another full walk.
fn advance_movement(map: &Map, units: &mut [Unit]) -> u32 {
    let before: Vec<(i32, i32)> = units.iter().map(|u| (u.x, u.y)).collect();
    let mut claimed: Vec<(i32, i32)> = Vec::new();
    let mut blocked = 0u32;
    for (i, unit) in units.iter_mut().enumerate() {
        if unit.path.is_empty() {
            unit.progress = 0;
            continue;
        }
        unit.progress = unit.progress.saturating_add(1);
        if unit.progress < unit.kind.move_ticks() {
            continue;
        }
        let Some(&next) = unit.path.front() else {
            continue;
        };
        let held = before
            .iter()
            .enumerate()
            .any(|(j, tile)| j != i && *tile == next);
        if held || claimed.contains(&next) || !map.walkable(next.0, next.1) {
            unit.progress = unit.kind.move_ticks().saturating_sub(1);
            blocked = blocked.saturating_add(1);
            continue;
        }
        unit.progress = 0;
        unit.path.pop_front();
        unit.x = next.0;
        unit.y = next.1;
        claimed.push(next);
    }
    blocked
}

fn strike(tick: u32, units: &mut Vec<Unit>, shots: &mut Vec<Shot>) {
    let ids: Vec<u32> = units.iter().map(|u| u.id).collect();
    for id in ids {
        let Some(i) = units.iter().position(|u| u.id == id) else {
            continue;
        };
        let Some(target) = units[i].target else {
            continue;
        };
        let Some(j) = units.iter().position(|u| u.id == target) else {
            units[i].target = None;
            continue;
        };
        if units[i].reload > 0 {
            units[i].reload -= 1;
            if units[i].reload > 0 {
                continue;
            }
        }
        let reach = units[i].kind.range();
        let dx = units[i].x.abs_diff(units[j].x);
        let dy = units[i].y.abs_diff(units[j].y);
        if dx.saturating_add(dy) > reach {
            continue;
        }
        let kind = units[i].kind;
        let damage = kind.attack().saturating_sub(units[j].kind.defence());
        units[i].reload = kind.attack_period();
        if kind.flight() == 0 {
            units[j].hp = units[j].hp.saturating_sub(damage);
            units.retain(|u| u.hp > 0);
        } else {
            shots.push(Shot {
                from: id,
                target,
                due: tick.saturating_add(kind.flight()),
                damage,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{UnitKind, UNIT_KIND_COUNT};
    use crate::economy::{HouseKind, Ware};
    use crate::map::Terrain;
    use crate::sim::{Command, Refusal, Sim};

    fn spawn(sim: &mut Sim, kind: UnitKind, side: u8, x: i32, y: i32) -> u32 {
        sim.apply_for(side, Command::SpawnUnit { kind, side, x, y })
            .expect("spawn");
        sim.units().last().expect("a unit").id
    }

    #[test]
    fn the_table_covers_every_kind() {
        assert_eq!(UnitKind::ALL.len(), UNIT_KIND_COUNT);
        for (i, kind) in UnitKind::ALL.into_iter().enumerate() {
            assert_eq!(usize::from(kind.index()), i);
            assert!(kind.hp() > 0);
            assert!(kind.move_ticks() > 0);
            assert!(kind.attack_period() > 0);
            assert!(!kind.cost().is_empty());
        }
    }

    #[test]
    fn spawn_refuses_water_occupied_and_out_of_bounds() {
        let mut sim = Sim::new(12, 12);
        assert!(sim.set_terrain(3, 3, Terrain::Water));
        assert_eq!(
            sim.apply(Command::SpawnUnit {
                kind: UnitKind::Worker,
                side: 0,
                x: 3,
                y: 3
            }),
            Err(Refusal::NotBuildable),
            "water"
        );
        assert_eq!(
            sim.apply(Command::SpawnUnit {
                kind: UnitKind::Worker,
                side: 0,
                x: 99,
                y: 0
            }),
            Err(Refusal::OutOfBounds),
            "outside the map"
        );
        spawn(&mut sim, UnitKind::Worker, 0, 5, 5);
        assert_eq!(
            sim.apply_for(
                1,
                Command::SpawnUnit {
                    kind: UnitKind::Swordsman,
                    side: 1,
                    x: 5,
                    y: 5
                }
            ),
            Err(Refusal::Occupied),
            "another unit stands there"
        );
        assert_eq!(
            sim.apply_for(
                2,
                Command::SpawnUnit {
                    kind: UnitKind::Worker,
                    side: 2,
                    x: 6,
                    y: 5
                }
            ),
            Err(Refusal::BadSide),
            "only sides 0 and 1 exist"
        );
        for x in 0..6 {
            sim.apply(Command::PlaceRoad { x, y: 11 }).expect("road");
        }
        sim.apply(Command::PlaceHouse {
            kind: HouseKind::Warehouse,
            x: 0,
            y: 8,
        })
        .expect("warehouse");
        assert_eq!(
            sim.apply_for(
                1,
                Command::SpawnUnit {
                    kind: UnitKind::Worker,
                    side: 1,
                    x: 1,
                    y: 8
                }
            ),
            Err(Refusal::Occupied),
            "a house stands there"
        );
    }

    #[test]
    fn a_swordsman_crosses_a_tile_every_four_ticks() {
        let mut sim = Sim::new(12, 12);
        let id = spawn(&mut sim, UnitKind::Swordsman, 0, 1, 5);
        assert_eq!(sim.stock(Ware::Wood), 37, "three wood for the swordsman");
        sim.apply(Command::MoveUnit { id, x: 4, y: 5 })
            .expect("move");
        for _ in 0..11 {
            sim.tick();
        }
        let unit = sim.unit(id).expect("unit");
        assert_eq!((unit.x, unit.y), (3, 5), "tick 11 is one short");
        assert_eq!(unit.progress, 3, "three ticks into the last tile");
        sim.tick();
        let unit = sim.unit(id).expect("unit");
        assert_eq!((unit.x, unit.y), (4, 5), "tick 12 lands the third tile");
        assert_eq!(unit.progress, 0);
        assert!(unit.path.is_empty());
        sim.tick();
        assert_eq!(sim.unit(id).map(|u| (u.x, u.y)), Some((4, 5)));
    }

    #[test]
    fn two_units_ordered_to_one_tile_end_on_different_tiles() {
        let mut sim = Sim::new(7, 1);
        let first = spawn(&mut sim, UnitKind::Swordsman, 0, 1, 0);
        let second = spawn(&mut sim, UnitKind::Swordsman, 1, 5, 0);
        sim.apply(Command::MoveUnit {
            id: first,
            x: 3,
            y: 0,
        })
        .expect("the first walk");
        sim.apply_for(
            1,
            Command::MoveUnit {
                id: second,
                x: 3,
                y: 0,
            },
        )
        .expect("the second walk");
        for _ in 0..12 {
            sim.tick();
        }
        let first = sim.unit(first).expect("the first unit");
        let second = sim.unit(second).expect("the second unit");
        assert_eq!((first.x, first.y), (3, 0), "the lower id took the tile");
        assert_eq!((second.x, second.y), (4, 0), "the other waits beside it");
        assert_ne!(
            (first.x, first.y),
            (second.x, second.y),
            "two bodies never share a tile"
        );
        assert!(
            sim.blocked_moves() >= 1,
            "the second entry kept waiting: {}",
            sim.blocked_moves()
        );
    }

    #[test]
    fn a_tile_vacated_this_tick_is_free_next_tick_twice() {
        #[derive(Debug, PartialEq, Eq)]
        struct Snapshot {
            waited: [(i32, i32); 2],
            entered: [(i32, i32); 2],
            hash: u64,
        }

        fn play() -> Snapshot {
            let mut sim = Sim::new(6, 1);
            let walker = spawn(&mut sim, UnitKind::Swordsman, 0, 1, 0);
            sim.apply(Command::MoveUnit {
                id: walker,
                x: 3,
                y: 0,
            })
            .expect("the path is planned before the other body stands on it");
            let other = spawn(&mut sim, UnitKind::Swordsman, 1, 2, 0);
            sim.apply_for(
                1,
                Command::MoveUnit {
                    id: other,
                    x: 5,
                    y: 0,
                },
            )
            .expect("the other walks off");
            for _ in 0..4 {
                sim.tick();
            }
            let walked = sim.unit(walker).expect("the walker");
            let moved = sim.unit(other).expect("the other");
            let waited = [(walked.x, walked.y), (moved.x, moved.y)];
            sim.tick();
            let walked = sim.unit(walker).expect("the walker");
            let moved = sim.unit(other).expect("the other");
            let entered = [(walked.x, walked.y), (moved.x, moved.y)];
            Snapshot {
                waited,
                entered,
                hash: sim.hash(),
            }
        }
        let first = play();
        let second = play();
        assert_eq!(first, second, "the swap resolves the same way twice");
        assert_eq!(first.waited, [(1, 0), (3, 0)], "the vacated tile waits");
        assert_eq!(
            first.entered,
            [(2, 0), (3, 0)],
            "the walker enters behind it"
        );
    }

    #[test]
    fn a_unit_blocked_by_a_body_arrives_later_and_arrives() {
        let mut sim = Sim::new(8, 1);
        let walker = spawn(&mut sim, UnitKind::Swordsman, 0, 0, 0);
        sim.apply(Command::MoveUnit {
            id: walker,
            x: 5,
            y: 0,
        })
        .expect("the walk is planned on an empty row");
        let ahead = spawn(&mut sim, UnitKind::Swordsman, 1, 2, 0);
        for _ in 0..4 {
            sim.tick();
        }
        sim.apply_for(
            1,
            Command::MoveUnit {
                id: ahead,
                x: 7,
                y: 0,
            },
        )
        .expect("the other walks the same way");
        let mut arrived = 0;
        for tick in 5..=40 {
            sim.tick();
            if sim
                .unit(walker)
                .is_some_and(|unit| (unit.x, unit.y) == (5, 0))
            {
                arrived = tick;
                break;
            }
        }
        assert_eq!(arrived, 21, "five tiles at four ticks, one tick lost");
        assert!(
            sim.unit(walker).is_some_and(|unit| unit.path.is_empty()),
            "the walker arrived"
        );
        assert_eq!(sim.blocked_moves(), 1, "one entry had to wait");
    }

    #[test]
    fn the_tile_fight_receipt_holds_twice() {
        fn play() -> (u64, usize, u32) {
            let mut sim = Sim::new(12, 6);
            let first = spawn(&mut sim, UnitKind::Swordsman, 0, 1, 3);
            let second = spawn(&mut sim, UnitKind::Swordsman, 1, 9, 3);
            sim.apply(Command::MoveUnit {
                id: first,
                x: 5,
                y: 3,
            })
            .expect("the first walk");
            sim.apply_for(
                1,
                Command::MoveUnit {
                    id: second,
                    x: 5,
                    y: 3,
                },
            )
            .expect("the second walk");
            for _ in 0..16 {
                sim.tick();
            }
            sim.apply(Command::Attack {
                id: first,
                target: second,
            })
            .expect("the first strike");
            sim.apply_for(
                1,
                Command::Attack {
                    id: second,
                    target: first,
                },
            )
            .expect("the second strike");
            for _ in 0..80 {
                sim.tick();
            }
            (sim.hash(), sim.units().len(), sim.blocked_moves())
        }
        let first = play();
        let second = play();
        assert_eq!(first, second, "the same fight twice, same receipt");
        assert_eq!(first.1, 1, "one swordsman stands");
        assert_eq!(first.2, 1, "the shared middle tile blocked one entry");
    }

    #[test]
    fn a_duel_ends_on_tick_21_with_the_swordsman_alive() {
        let mut sim = Sim::new(10, 10);
        let swordsman = spawn(&mut sim, UnitKind::Swordsman, 0, 4, 4);
        let archer = spawn(&mut sim, UnitKind::Archer, 1, 5, 4);
        sim.apply(Command::Attack {
            id: swordsman,
            target: archer,
        })
        .expect("order");
        sim.apply_for(
            1,
            Command::Attack {
                id: archer,
                target: swordsman,
            },
        )
        .expect("order");
        for _ in 0..20 {
            sim.tick();
        }
        assert_eq!(
            sim.unit(archer).map(|u| u.hp),
            Some(4),
            "two sword hits by tick 20"
        );
        assert_eq!(
            sim.unit(swordsman).map(|u| u.hp),
            Some(22),
            "two arrows landed by tick 20"
        );
        sim.tick();
        assert!(
            sim.unit(archer).is_none(),
            "the third hit ends the duel on tick 21"
        );
        assert_eq!(sim.unit(swordsman).map(|u| u.hp), Some(22));
    }

    #[test]
    fn an_arrow_lands_four_ticks_after_the_shot() {
        let mut sim = Sim::new(10, 10);
        let archer = spawn(&mut sim, UnitKind::Archer, 0, 0, 3);
        let swordsman = spawn(&mut sim, UnitKind::Swordsman, 1, 2, 3);
        sim.apply(Command::Attack {
            id: archer,
            target: swordsman,
        })
        .expect("order");
        for _ in 0..4 {
            sim.tick();
        }
        assert_eq!(
            sim.unit(swordsman).map(|u| u.hp),
            Some(30),
            "the arrow is still in the air"
        );
        assert_eq!(sim.shots().len(), 1);
        sim.tick();
        assert_eq!(
            sim.unit(swordsman).map(|u| u.hp),
            Some(26),
            "six attack less two defence"
        );
        assert!(sim.shots().is_empty());
    }

    #[test]
    fn a_swordsman_two_tiles_away_strikes_nothing() {
        let mut sim = Sim::new(10, 10);
        let melee = spawn(&mut sim, UnitKind::Swordsman, 0, 2, 2);
        let archer = spawn(&mut sim, UnitKind::Archer, 1, 4, 2);
        sim.apply(Command::Attack {
            id: melee,
            target: archer,
        })
        .expect("order");
        for _ in 0..30 {
            sim.tick();
        }
        assert_eq!(sim.unit(archer).map(|u| u.hp), Some(18));
        assert_eq!(sim.unit(melee).map(|u| u.reload), Some(0));
    }

    #[test]
    fn attack_refuses_an_unknown_unit_and_itself() {
        let mut sim = Sim::new(8, 8);
        let id = spawn(&mut sim, UnitKind::Worker, 0, 2, 2);
        assert_eq!(
            sim.apply(Command::Attack { id, target: 99 }),
            Err(Refusal::NoUnit)
        );
        assert_eq!(
            sim.apply(Command::Attack { id, target: id }),
            Err(Refusal::NoUnit)
        );
        assert_eq!(
            sim.apply(Command::MoveUnit { id: 99, x: 3, y: 3 }),
            Err(Refusal::NoUnit)
        );
    }

    fn skirmish() -> Sim {
        let mut sim = Sim::new(14, 12);
        for x in 0..8 {
            sim.apply(Command::PlaceRoad { x, y: 11 }).expect("road");
        }
        sim.apply(Command::PlaceHouse {
            kind: HouseKind::Warehouse,
            x: 0,
            y: 8,
        })
        .expect("warehouse");
        let swordsman = spawn(&mut sim, UnitKind::Swordsman, 0, 1, 1);
        let archer = spawn(&mut sim, UnitKind::Archer, 1, 6, 1);
        sim.apply(Command::MoveUnit {
            id: swordsman,
            x: 5,
            y: 1,
        })
        .expect("move");
        sim.apply_for(
            1,
            Command::Attack {
                id: archer,
                target: swordsman,
            },
        )
        .expect("order");
        sim
    }

    #[test]
    fn same_commands_same_hash_with_units() {
        let mut a = skirmish();
        let mut b = skirmish();
        for _ in 0..40 {
            a.tick();
            b.tick();
        }
        assert_eq!(a.hash(), b.hash());
        assert_eq!(
            a.unit(0).map(|u| u.hp),
            Some(18),
            "three arrows landed by tick 40"
        );
        assert_eq!(a.units().len(), 2);
    }
}
