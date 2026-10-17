//! Print this build's spine parity column (ARC10): the legacy arena step and
//! the spine step over 20 seeded 600-tick tapes, as one scorecard line.
//! Exit 0 when every tape matched, 1 otherwise. `--seeded-wall` plays the
//! spine side around a wall the legacy step lacks: the canary that proves the
//! gate can fail. `tools/proof.mjs` reads both runs.

use engine_arena::world::MatchWorld;
use engine_proof::spine_parity::{column, seeded_wall};

fn main() {
    let seeded = std::env::args().any(|arg| arg == "--seeded-wall");
    let world = if seeded {
        seeded_wall()
    } else {
        MatchWorld::empty()
    };
    let column = column(&world);
    println!("{}", column.line());
    if !column.pass() {
        std::process::exit(1);
    }
}
