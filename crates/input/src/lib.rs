//! input: deterministic action map for fixed-tick sim. Bits hash into the tick hash.

#![warn(missing_docs)]

pub mod action_state;
pub mod bindings;
pub mod fighter;
pub mod gestures;
pub mod map;

pub use bindings::{
    apply_deadzone, Binding, Bindings, BoundAction, InputConfig, PhysicalButton, ACTION_COUNT,
    DEFAULT_DEADZONE, KIND_KEY, KIND_MOUSE, KIND_PAD,
};

pub use fighter::{Buttons, FighterError, FighterId, FighterInput, FighterTape, MAX_PARTICIPANTS};
pub use gestures::{
    Gesture, GestureDetector, DOUBLE_TAP_MAX_GAP_TICKS, HOLD_MIN_HELD_TICKS, TAP_MAX_HELD_TICKS,
    TAP_MIN_HELD_TICKS,
};
pub use map::{hash_bits, Action, Chord, Frame, Input, InputMap, Key, RepeatLast};

/// Neutral alias for [`FighterTape`]: one bout's input tape. Cross-genre
/// consumers (proof, playtest, editor) use this name so genre-neutral code
/// never names a fighter type; the canonical type keeps its name for existing
/// call sites. Identical layout, zero-cost.
pub type BoutTape = FighterTape;

/// Neutral alias for [`FighterInput`]: one participant's tick input.
/// Cross-genre consumers use this name so genre-neutral code never names a
/// fighter type; identical layout, zero-cost.
pub type ParticipantInput = FighterInput;

/// Neutral alias for [`FighterId`]: stable participant slot in
/// `0..MAX_PARTICIPANTS`. Cross-genre consumers use this name so
/// genre-neutral code never names a fighter type; identical layout, zero-cost.
pub type ParticipantId = FighterId;

/// Neutral alias for [`FighterError`]: participant contract failures.
/// Cross-genre consumers use this name; identical layout, zero-cost.
pub type ParticipantError = FighterError;

#[cfg(test)]
mod root_alias_tests {
    use super::*;

    fn participant(tick: u32, slot: u8) -> ParticipantInput {
        ParticipantInput {
            tick,
            fighter: ParticipantId::new(slot).expect("slot must be valid"),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        }
    }

    #[test]
    fn neutral_tape_validates_sorted_inputs() {
        let tape: BoutTape = FighterTape(vec![participant(0, 0), participant(1, 0)]);
        tape.validate().expect("sorted tape validates");
        assert_eq!(BoutTape::default().0.len(), 0);
    }

    #[test]
    fn neutral_tape_rejects_unsorted_and_duplicate() {
        let (a, b) = (participant(0, 0), participant(1, 0));
        let flipped: BoutTape = FighterTape(vec![b, a]);
        assert_eq!(flipped.validate(), Err(ParticipantError::UnsortedTape(1)));
        let dupe: BoutTape = FighterTape(vec![a, a]);
        assert_eq!(dupe.validate(), Err(ParticipantError::DuplicateInput(1)));
    }

    #[test]
    fn participant_slot_gate_matches_cap() {
        assert_eq!(MAX_PARTICIPANTS, 16);
        assert!(ParticipantId::new(0).is_some());
        assert!(ParticipantId::new(15).is_some());
        assert_eq!(ParticipantId::new(16), None);
        assert_eq!(ParticipantId::new(u8::MAX), None);
        let mut bad = participant(0, 0);
        bad.fighter = FighterId(16);
        assert_eq!(bad.validate(), Err(ParticipantError::UnknownFighter(16)));
    }

    #[test]
    fn participant_input_rejects_bad_ranges() {
        let mut wish = participant(0, 0);
        wish.mx = 101;
        assert_eq!(wish.validate(), Err(ParticipantError::WishOutOfRange(101)));
        let mut facing = participant(0, 0);
        facing.facing = 18_001;
        assert_eq!(
            facing.validate(),
            Err(ParticipantError::FacingOutOfRange(18_001))
        );
        let mut buttons = participant(0, 0);
        buttons.buttons = Buttons(0x08);
        assert_eq!(
            buttons.validate(),
            Err(ParticipantError::UnknownButtons(0x08))
        );
    }

    #[test]
    fn root_reexports_hash_and_deadzone() {
        assert_eq!(ACTION_COUNT, 7);
        assert_eq!(DEFAULT_DEADZONE, 8);
        assert_eq!(hash_bits(0), hash_bits(0));
        assert_ne!(hash_bits(0), hash_bits(1));
        assert_eq!(apply_deadzone(0, 0, DEFAULT_DEADZONE), (0, 0));
        assert_eq!(apply_deadzone(5, 5, DEFAULT_DEADZONE), (0, 0));
        assert_eq!(apply_deadzone(100, 0, DEFAULT_DEADZONE), (100, 0));
    }
}
