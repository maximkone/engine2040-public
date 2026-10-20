//! Tiles: two swordsmen eight tiles apart close in on open ground; one tile holds one body.
//!
//! Run: `cargo run -p kam2-sim --example tiles`
//! One run plays the meeting twice and prints both receipt lines; the hash must agree.
//! `approached` is the fall of the taxicab gap, `blocked` counts tile entries that had to
//! wait for a body.
//!
//! Receipts, old -> new (SIM2): the town `6d282b76f7a45609` -> `6d282b76f7a45609` and the
//! mayor `9da53332e624d075` -> `9da53332e624d075` (neither walks a unit). The order
//! skirmish in `kam2-game` keeps its hash `5543378eae5463ff` -> `5543378eae5463ff` but its
//! line moves to `approached=7`: its two swordsmen used to enter the middle tile together
//! and now stop beside it, and the fallen swordsman is not in the digest. This example
//! prints `KAM2 TILES OK tick=16 approached=7 blocked=1 hash=ee47e4fe812e402f`.

use kam2_sim::{Command, Sim, Unit, UnitKind};

/// Start gap in tiles.
const GAP: i32 = 8;
/// Ticks one meeting may play before it counts as a stall.
const TICKS: u32 = 200;
/// Neighbour order for every tie: west, east, north, south.
const NEIGHBOURS: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];

/// One finished meeting, as the receipt needs it.
struct Receipt {
    tick: u32,
    approached: i32,
    blocked: u32,
    hash: u64,
}

/// Two swordsmen eight tiles apart on open ground.
fn start() -> Sim {
    let mut sim = Sim::new(12, 6);
    for (side, x, y) in [(0u8, 1, 3), (1, 9, 3)] {
        sim.apply_for(
            side,
            Command::SpawnUnit {
                kind: UnitKind::Swordsman,
                side,
                x,
                y,
            },
        )
        .expect("the meeting fields both swordsmen");
    }
    sim
}

/// Taxicab tiles between the two swordsmen, while both live.
fn gap(sim: &Sim) -> Option<i32> {
    let first = sim.unit(0)?;
    let second = sim.unit(1)?;
    Some((first.x - second.x).abs() + (first.y - second.y).abs())
}

/// The free walkable tile beside `target` nearest to `unit`, ties by [`NEIGHBOURS`] order.
fn beside(sim: &Sim, unit: &Unit, target: &Unit) -> Option<(i32, i32)> {
    let mut best: Option<(i32, (i32, i32))> = None;
    for (dx, dy) in NEIGHBOURS {
        let tile = (target.x + dx, target.y + dy);
        if !sim.map().walkable(tile.0, tile.1) {
            continue;
        }
        if sim
            .units()
            .iter()
            .any(|other| other.id != unit.id && (other.x, other.y) == tile)
        {
            continue;
        }
        let distance = (tile.0 - unit.x).abs() + (tile.1 - unit.y).abs();
        if best.is_none_or(|(seen, _)| distance < seen) {
            best = Some((distance, tile));
        }
    }
    best.map(|(_, tile)| tile)
}

/// One meeting: order each free walker beside the other, then tick until they are adjacent.
fn play() -> Receipt {
    let mut sim = start();
    let start_gap = gap(&sim).expect("both swordsmen live");
    let mut closest = start_gap;
    while gap(&sim).is_some_and(|distance| distance > 1) && sim.ticks() < TICKS {
        for id in [0u32, 1] {
            let Some(unit) = sim.unit(id) else {
                continue;
            };
            if !unit.path.is_empty() {
                continue;
            }
            let Some(target) = sim.unit(1 - id) else {
                continue;
            };
            let Some(goal) = beside(&sim, unit, target) else {
                continue;
            };
            let side = unit.side;
            sim.apply_for(
                side,
                Command::MoveUnit {
                    id,
                    x: goal.0,
                    y: goal.1,
                },
            )
            .expect("the open row is walkable");
        }
        sim.tick();
        if let Some(distance) = gap(&sim) {
            closest = closest.min(distance);
        }
    }
    Receipt {
        tick: sim.ticks(),
        approached: start_gap - closest,
        blocked: sim.blocked_moves(),
        hash: sim.hash(),
    }
}

/// One receipt line; both runs print the same line when the tile rule holds.
fn line(receipt: &Receipt) -> String {
    format!(
        "KAM2 TILES OK tick={} approached={} blocked={} hash={:016x}",
        receipt.tick, receipt.approached, receipt.blocked, receipt.hash
    )
}

fn main() {
    let first = play();
    let second = play();
    assert_eq!(first.tick, second.tick, "the same contact tick twice");
    assert_eq!(first.hash, second.hash, "the same state twice");
    assert_eq!(first.approached, GAP - 1, "the swordsmen stop adjacent");
    assert!(first.blocked > 0, "the shared beside tile made one wait");
    println!("{}", line(&first));
    println!("{}", line(&second));
}
