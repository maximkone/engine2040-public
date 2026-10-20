//! Headless town: one road, a warehouse, a farm, a mill and a bakery play 2000 ticks.
//!
//! Run: `cargo run -p kam2-sim --example town`
//! Prints the stock every 200 ticks and one final line with the state hash.
//!
//! The receipt hash moved with the side split (SIM1): `a7d73ecdbeba76e5` before,
//! `6d282b76f7a45609` now, because side 1's own starting stock is in the digest.
//! SIM2 left it unchanged: `6d282b76f7a45609` -> `6d282b76f7a45609`, because the town
//! walks no unit and the tile rule only changes the movement phase.

use kam2_sim::{Command, HouseKind, Sim, Ware};

fn town() -> Sim {
    let mut sim = Sim::new(16, 12);
    for x in 0..14 {
        if let Err(why) = sim.apply(Command::PlaceRoad { x, y: 6 }) {
            panic!("road {x} refused: {why:?}");
        }
    }
    for (kind, x, y) in [
        (HouseKind::Warehouse, 0, 3),
        (HouseKind::Farm, 4, 4),
        (HouseKind::Mill, 7, 4),
        (HouseKind::Bakery, 10, 4),
    ] {
        if let Err(why) = sim.apply(Command::PlaceHouse { kind, x, y }) {
            panic!("{} refused: {why:?}", kind.name());
        }
    }
    sim
}

fn main() {
    let mut sim = town();
    println!(
        "houses={} roads=14 grain={} flour={} bread={}",
        sim.houses().len(),
        sim.stock(Ware::Grain),
        sim.stock(Ware::Flour),
        sim.stock(Ware::Bread)
    );
    for _ in 0..2000 {
        sim.tick();
        if sim.ticks().is_multiple_of(200) {
            println!(
                "tick={} grain={} flour={} bread={}",
                sim.ticks(),
                sim.stock(Ware::Grain),
                sim.stock(Ware::Flour),
                sim.stock(Ware::Bread)
            );
        }
    }
    let hash = sim.hash();
    println!(
        "KAM2 TOWN OK ticks={} bread={} grain={} flour={} hash={hash:016x}",
        sim.ticks(),
        sim.stock(Ware::Bread),
        sim.stock(Ware::Grain),
        sim.stock(Ware::Flour)
    );
    assert!(sim.stock(Ware::Bread) > 0, "the town never baked bread");
}
