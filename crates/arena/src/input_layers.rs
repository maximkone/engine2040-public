//! input_layers: the input layer beside the tick hash (IN-L1).
//!
//! Consumer: the door's play verbs and the window shell read the layer
//! digest next to the tick hash; the recorder stores the layer per row.
//! Data owner: the caller owns the tape and the layer table
//! (`engine_input::layers::LayerTape`).
//! Units: fixed ticks and one digest per tape.
//! Determinism: the checkpoints are the ones [`run`](crate::run) gives for
//! the same tape; the layer digest is a pure function of the layer rows.
//! Errors: the tape's own rules; an invalid tape runs nothing.
//!
//! Boundary, and what it costs. The layer never enters the tick hash.
//! `docs/HASH-LAW.md` freezes the golden `d57b5c8ae3096614`, and the sim
//! bits already say "no buttons" for a menu tick, so the layer rides beside
//! the tape and is hashed on its own. The cost of that choice: inside the
//! sim a menu-gated tick and an idle duel tick hash the same, so a replay
//! cannot tell them apart without this digest. Putting the layer in the
//! tick hash needs a tape format version bump plus a golden migration, which
//! is an owner decision, not a lane call.

use crate::{ArenaConfig, Checkpoint};
use engine_input::layers::LayerTape;

/// Run a match over a layered tape: the checkpoints of [`run`](crate::run)
/// plus the layer digest. The layer never changes the checkpoints.
pub fn run_layered(config: &ArenaConfig, tape: &LayerTape) -> (Vec<Checkpoint>, u64) {
    (crate::run(config, tape.tape()), tape.digest())
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_input::bindings::ActionSet;
    use engine_input::{Buttons, FighterId, FighterInput, FighterTape};

    fn config() -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 120,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: engine_combat::tables::TABLES_VERSION_V1,
        }
    }

    fn tape(ticks: u32) -> FighterTape {
        let id = FighterId::new(0).expect("slot 0 legal");
        FighterTape(
            (0..ticks)
                .map(|tick| FighterInput {
                    tick,
                    fighter: id,
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: Buttons::NONE,
                })
                .collect(),
        )
    }

    /// The layer rides beside the hash: the same tape gives the same
    /// checkpoints whatever the layer says, and the digest says which layer
    /// ran.
    #[test]
    fn input_layers_layer_changes_the_digest_not_the_checkpoints() {
        let mut duel = LayerTape::new(tape(30));
        duel.set_layer(10, ActionSet::Duel).expect("row lands");
        let mut menu = LayerTape::new(tape(30));
        menu.set_layer(10, ActionSet::Menu).expect("row lands");

        let (duel_checkpoints, duel_digest) = run_layered(&config(), &duel);
        let (menu_checkpoints, menu_digest) = run_layered(&config(), &menu);
        assert_eq!(
            duel_checkpoints, menu_checkpoints,
            "the layer never moves the tick hash"
        );
        assert_ne!(duel_digest, menu_digest, "the digest names the layer");
        assert_eq!(duel_digest, duel.digest());
        // The plain run over the same tape gives the same checkpoints.
        assert_eq!(duel_checkpoints, crate::run(&config(), &tape(30)));
    }
}
