//! Rival-door parity: the playtest verbs other engines' doors have.
//!
//! Consumer: reviewers checking LF16 against the `godot-mcp` door
//! (`godot_game_time`, `godot_input`, `godot_runtime_state`; see the LF16 row
//! and `ai-native.md` verb gap). This file owns only the mapping table from a
//! rival verb to our play-pack verb; the verbs themselves live in `play.rs`
//! (`play.step_until`) and `play_ext.rs` (`input.inject`, `state.digest`).
//! Units: none (names only). Determinism: table order is fixed. Errors: none;
//! the test below fails when a mapped verb leaves the play pack.
use super::play_ext::{TOOL_INPUT_INJECT, TOOL_STATE_DIGEST};
use super::TOOL_PLAY_STEP_UNTIL;

/// One rival verb and the play-pack verb that answers it, in door order.
pub const RIVAL_PARITY: [(&str, &str); 3] = [
    ("godot_game_time", TOOL_PLAY_STEP_UNTIL),
    ("godot_input", TOOL_INPUT_INJECT),
    ("godot_runtime_state", TOOL_STATE_DIGEST),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs::PackRegistry;
    use crate::tool_capabilities::CapabilityCard;

    #[test]
    fn every_rival_verb_maps_to_a_live_play_pack_verb() {
        assert_eq!(RIVAL_PARITY.len(), 3, "one row per LF16 verb");
        let registry = PackRegistry::shipped();
        for (rival, ours) in RIVAL_PARITY {
            let spec = registry
                .verb(ours)
                .unwrap_or_else(|| panic!("rival `{rival}` maps to `{ours}`, which registers"));
            assert_eq!(
                spec.pack(),
                crate::packs::play::PACK,
                "{ours} stays in the play pack"
            );
            assert_eq!(
                spec.card(),
                CapabilityCard::Observe,
                "{ours} stays read-only scoped"
            );
            assert!(spec.readonly(), "{ours} writes nothing");
        }
    }
}
