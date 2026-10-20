//! Print one ossim screen as text: `cargo run -p engine-ossim --example screen`.
//!
//! Proof for OS6: the headless core now has a screen. The tape is fixed
//! (seed 7, stage FIRST BOOT), so the output is the same twice.

use engine_ossim::{Action, Screen, Sim, STAGES};

fn main() {
    let mut sim = Sim::new(7, STAGES[0]);
    let id = sim
        .state()
        .processes
        .iter()
        .flatten()
        .next()
        .map(|process| process.id);
    if let Some(id) = id {
        let _ = sim.tick(Some(Action::Assign {
            cpu: 0,
            process: id,
        }));
    }
    for _ in 0..600 {
        let _ = sim.tick(None);
    }
    print!("{}", Screen::render(&sim.state()).to_text());
}
