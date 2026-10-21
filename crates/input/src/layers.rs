//! layers: the input layer that owns each tick, hashed beside the tape.
//!
//! Consumer: the arena's layered run (`engine_arena::input_layers::run_layered`)
//! prints the layer digest next to the tick hash, and the window shell reads
//! the same stack (`engine_input::bindings::ActionSetStack`). Data owner:
//! [`LayerTape`] owns the tape plus the layer of each tick.
//! Units: fixed ticks and one byte per layer.
//! Determinism: the digest is FNV-1a 64 over the `(tick, layer byte)` pairs
//! in tick order, so the same layers always give the same digest.
//! Errors: [`LayerError`], never a panic.
//! Boundary: the layer never enters the tick hash. `docs/HASH-LAW.md` freezes
//! the golden `d57b5c8ae3096614`, and the sim bits already say "no buttons"
//! for a menu tick, so the layer rides beside the tape and is hashed on its
//! own. The cost of that choice is written down in the arena's
//! `input_layers` module.

use crate::bindings::ActionSet;
use crate::fighter::{FighterError, FighterTape};
use engine_core::Fnv64;

/// Why a layered tape refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayerError {
    /// The tape itself is not a valid tape.
    Tape(FighterError),
    /// Two rows name the same tick.
    DuplicateTick(u32),
    /// A row's tick is before the previous row's tick.
    UnorderedTick(u32),
}

/// One tape plus the layer that owns each tick.
///
/// A tick with no row is [`ActionSet::Duel`]: the layer table only records
/// the ticks where something else owned the buttons.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LayerTape {
    tape: FighterTape,
    layers: Vec<(u32, ActionSet)>,
}

impl LayerTape {
    /// A tape with no layer rows: every tick is Duel.
    #[must_use]
    pub fn new(tape: FighterTape) -> Self {
        Self {
            tape,
            layers: Vec::new(),
        }
    }

    /// The tape the layers ride on.
    #[must_use]
    pub fn tape(&self) -> &FighterTape {
        &self.tape
    }

    /// Record the layer of one tick. Rows stay tick-sorted; a tick that
    /// already has a row is refused, so the table never holds two answers
    /// for one tick.
    pub fn set_layer(&mut self, tick: u32, set: ActionSet) -> Result<(), LayerError> {
        if let Some(last) = self.layers.last() {
            if tick < last.0 {
                return Err(LayerError::UnorderedTick(tick));
            }
            if tick == last.0 {
                return Err(LayerError::DuplicateTick(tick));
            }
        }
        self.layers.push((tick, set));
        Ok(())
    }

    /// The layer of one tick; [`ActionSet::Duel`] when the tick has no row.
    #[must_use]
    pub fn layer_at(&self, tick: u32) -> ActionSet {
        self.layers
            .iter()
            .find(|(row, _)| *row == tick)
            .map_or(ActionSet::Duel, |(_, set)| *set)
    }

    /// The layer rows, in tick order.
    #[must_use]
    pub fn layers(&self) -> &[(u32, ActionSet)] {
        &self.layers
    }

    /// FNV-1a 64 over the `(tick, layer byte)` pairs in tick order: the same
    /// layers always give the same digest, and a different layer gives a
    /// different one. The tape bits are not part of this digest.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut fold = Fnv64::new();
        fold.write_len(self.layers.len());
        for (tick, set) in &self.layers {
            fold.write_u32(*tick);
            // `ActionSet::byte` is the fixed table order (`ActionSet::ALL`), so
            // it is the byte the digest folds: Duel 0, Menu 1, Pause 2.
            fold.write_u8(set.byte());
        }
        fold.finish()
    }

    /// The tape validates and the layer rows are sorted and unique.
    pub fn validate(&self) -> Result<(), LayerError> {
        self.tape.validate().map_err(LayerError::Tape)?;
        let mut previous: Option<u32> = None;
        for (tick, _) in &self.layers {
            if let Some(last) = previous {
                if *tick == last {
                    return Err(LayerError::DuplicateTick(*tick));
                }
                if *tick < last {
                    return Err(LayerError::UnorderedTick(*tick));
                }
            }
            previous = Some(*tick);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fighter::{Buttons, FighterId, FighterInput};

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

    /// The layer digest changes with the layer and repeats with the same one.
    #[test]
    fn layers_digest_follows_the_layer_not_the_clock() {
        let mut duel = LayerTape::new(tape(4));
        duel.set_layer(2, ActionSet::Duel).expect("row lands");
        let mut menu = LayerTape::new(tape(4));
        menu.set_layer(2, ActionSet::Menu).expect("row lands");
        assert_ne!(duel.digest(), menu.digest(), "the layer is in the digest");
        assert_eq!(duel.digest(), duel.digest(), "the same layers agree");
        assert_eq!(duel.layer_at(2), ActionSet::Duel);
        assert_eq!(menu.layer_at(2), ActionSet::Menu);
        assert_eq!(
            menu.layer_at(3),
            ActionSet::Duel,
            "a tick with no row is the duel layer"
        );
    }

    /// A tick may hold one layer row only, and the rows stay tick-sorted.
    #[test]
    fn layers_refuse_a_duplicate_or_an_unordered_row() {
        let mut layered = LayerTape::new(tape(4));
        layered.set_layer(2, ActionSet::Menu).expect("row lands");
        assert_eq!(
            layered.set_layer(2, ActionSet::Pause).err(),
            Some(LayerError::DuplicateTick(2))
        );
        assert_eq!(
            layered.set_layer(1, ActionSet::Pause).err(),
            Some(LayerError::UnorderedTick(1))
        );
        layered.validate().expect("the table validates");
    }

    /// The tape under the layers still has to be a valid tape.
    #[test]
    fn layers_validate_refuses_a_broken_tape() {
        let mut broken = FighterTape::default();
        broken.0.push(FighterInput {
            tick: 0,
            fighter: FighterId::new(0).expect("slot 0 legal"),
            mx: 120,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        });
        let layered = LayerTape::new(broken);
        let outcome = layered.validate();
        assert!(
            matches!(outcome, Err(LayerError::Tape(_))),
            "the wish range is the tape's own rule"
        );
    }
}
