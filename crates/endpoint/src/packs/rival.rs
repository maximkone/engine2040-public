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

/// One shared core for both lookup directions: the table loop lives here
/// once, thin callers below only pick which lane to match and which to
/// return. Pure, no allocation, never panics.
fn find_parity<F>(matches: F) -> Option<(&'static str, &'static str)>
where
    F: FnMut(&(&'static str, &'static str)) -> bool,
{
    RIVAL_PARITY.iter().copied().find(matches)
}

/// Resolve a rival verb name to our play-pack verb, if it is one of the three.
///
/// Caller: `crate::insight::mcp_tools::ServerState::api_describe` resolves
/// rival aliases before the index lookup, so `godot_game_time` reads as
/// `play.step_until`. Pure, no allocation, never panics.
pub fn verb_for_rival(rival: &str) -> Option<&'static str> {
    find_parity(|(name, _)| *name == rival).map(|(_, ours)| ours)
}

/// Resolve our play-pack verb to its rival alias, if any.
///
/// Caller: reviewers checking LF16 parity; the door seeds the index with our
/// verb names and this maps them back to the rival door. Pure, never panics.
pub fn rival_for(verb: &str) -> Option<&'static str> {
    find_parity(|(_, ours)| *ours == verb).map(|(name, _)| name)
}

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

    #[test]
    fn rival_helpers_resolve_both_ways_and_refuse_unknown() {
        // Behaviour: each rival reads as its play-pack verb and back.
        // Caller: `ServerState::api_describe` resolves rivals through
        // `verb_for_rival`; reviewers map back through `rival_for`.
        for (rival, ours) in RIVAL_PARITY {
            assert_eq!(verb_for_rival(rival), Some(ours), "{rival} resolves");
            assert_eq!(rival_for(ours), Some(rival), "{ours} maps back");
            assert_eq!(
                verb_for_rival(rival),
                verb_for_rival(rival),
                "resolves twice"
            );
        }
        // Failure: unknown names on either side refuse as `None`, never panic.
        assert_eq!(verb_for_rival("godot_unknown"), None);
        assert_eq!(verb_for_rival("play.step_until_extra"), None);
        assert_eq!(verb_for_rival(""), None);
        assert_eq!(rival_for("play.step_until_extra"), None);
        assert_eq!(rival_for(""), None);
    }
}
