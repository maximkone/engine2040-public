//! EA6: the door's per-caller capability token.
//!
//! Consumer: [`super::startup::build_server`] mints exactly one token per door
//! session from the host's scope policy ([`ScopePolicy::ops`]) instead of
//! `Capability::unrestricted`, so the shipped door grants the classes the host
//! policy names, not every class. The tool cards stay the first gate
//! ([`super::tool_capabilities::card_for_tool`]): a `Deny` name refuses at the
//! dispatch seam, and a write reaches [`check_write`](super::tool_capabilities::check_write)
//! under the token this module mints.
//!
//! Units: session ids are dense door sessions, the edit budget counts ops.
//! Determinism: the same policy always mints the same token on every machine;
//! no wall clock, no float, no model.
//! Errors: a policy that does not parse never reaches this module; startup
//! refuses with `StartupError::BadPolicy` before the door opens.

use engine_editor::capability::Capability;

use super::tool_capabilities::ScopePolicy;

/// Remaining op budget one door session starts with. The gate's limit is the
/// policy's op classes and the EXPLAIN caps, not a session-wide op count, so
/// the budget stays inexhaustible exactly like the old unrestricted token.
pub const DOOR_EDIT_BUDGET: u32 = u32::MAX;

/// Mint one door session's capability token from the host's scope policy.
///
/// The op mask is the policy's grant; the object scope is full (empty
/// prefixes), matching the door's single-client session, and a later policy
/// line can narrow it without changing this call.
pub fn token_from_policy(policy: &ScopePolicy, session: u64) -> Capability {
    Capability::new(session, policy.ops(), Vec::new(), DOOR_EDIT_BUDGET)
}

#[cfg(test)]
#[path = "door_token_tests.rs"]
mod tests;
