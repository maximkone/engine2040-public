//! Headless mayor town: the AI mayor lays the roads, builds the chain and fields a guard
//! for 3000 ticks.
//!
//! Run: `cargo run -p kam2-sim --example mayor_town`
//! Prints the town every 500 ticks and one final receipt line.
//!
//! The receipt hash moved with the side split (SIM1): `82295fecdeefed91` before,
//! `9da53332e624d075` now, because side 1's own starting stock is in the digest.

use kam2_sim::{HouseKind, Mayor, Sim, Ware};

/// Ticks played.
const TICKS: u32 = 3000;
/// Ticks between two decision rounds.
const ROUND: u32 = 25;

fn main() {
    let mayor = Mayor::new((20, 12));
    let mut sim = Sim::new(24, 18);
    while sim.ticks() < TICKS {
        for cmd in mayor.decide(&sim) {
            if let Err(why) = sim.apply(cmd) {
                panic!("mayor command {cmd:?} refused: {why:?}");
            }
        }
        for _ in 0..ROUND {
            sim.tick();
        }
        if sim.ticks().is_multiple_of(500) {
            println!(
                "tick={} houses={} bread={} units={} hash={:016x}",
                sim.ticks(),
                sim.houses().len(),
                sim.stock(Ware::Bread),
                sim.units().len(),
                sim.hash()
            );
        }
    }
    let houses = sim.houses().len();
    let bread = sim.stock(Ware::Bread);
    let units = sim.units().len();
    let hash = sim.hash();
    println!(
        "KAM2 MAYOR OK tick={} houses={houses} bread={bread} units={units} hash={hash:016x}",
        sim.ticks()
    );
    assert!(
        sim.houses().iter().any(|h| h.kind == HouseKind::Warehouse),
        "the mayor never placed a warehouse"
    );
    assert!(bread > 0, "the town never baked bread");
}
