//! Shared guard block for tool writes: one format gate plus one
//! policy gate every mutating verb passes before it stages anything.
//!
//! Consumer: the MCP door writes (`tx.preview` / `tx.apply` through
//! `tool_capabilities`, `scene.place` through `scene_tools`) and any future
//! pack verb that mutates the document. The per-tool fronts delegate their
//! shape, tier and intent bounds here so every verb refuses the same way. Data owner stays
//! the editor command service; this module owns only the pure pre-checks.
//! Units: ops for batches, bytes for ids and intent text. Budget: no
//! allocation beyond one small message `String` per refusal; the gate runs at
//! the door, outside the fixed tick. Determinism: same inputs always seal the
//! same verdict on every machine; integer compares only, no wall clock, no
//! float. Errors: [`GuardError`] carries the machine `code` plus a message
//! that always names the legal values and one recovery, and converts into a
//! tool error without losing either.
//!
//! PORT (ideas reimplemented, zero bytes vendored): flax-mcp
//! `src/FlaxMcp.Core/Core/Tooling/ToolGuards.cs:40-84` (MIT, Maxim's own
//! code): `RequireIntent` (every write names its intent) plus
//! `RequireDryRunId` (the dry-run receipt id rides the `rcpt_` prefix under
//! a 512 cap) plus the policy gate (mutating writes admit, destructive ones
//! need an elevated session) plus the args/intent length caps.
//!
//! The two verbs behind the gate: `scene.place` ([`VerbClass::Mutating`], it
//! moves staged roots only) and `tx.apply` ([`VerbClass::Destructive`], the
//! delete-capable apply path; removes additionally need the elevated tier).
//! [`verb_class`] names them; anything else is `None`, so an unclassified
//! verb fails closed until it is classified here.
//!
//! Shape note: the op-count bound mirrors the transport bound
//! (`mcp::MAX_TX_OPS`, 256) without depending on it, so this module stays
//! std-only and its tests run without the `mcp` feature. The destructive
//! tier mirrors `tool_capabilities::DESTRUCTIVE_TIER` (3) the same way.

/// Required prefix for every dry-run receipt id (`RequireDryRunId`).
pub const DRY_RUN_ID_PREFIX: &str = "rcpt_";
/// Longest dry-run receipt id in bytes, prefix included.
pub const DRY_RUN_ID_MAX_BYTES: usize = 512;
/// Longest write intent text in bytes (`RequireIntent` length cap).
pub const INTENT_MAX_BYTES: usize = 2048;
/// Most ops per write batch (mirrors the transport bound, 256).
pub const ARGS_MAX_OPS: usize = 256;
/// Session tier that may run destructive verbs (mirrors the door tier, 3).
pub const DESTRUCTIVE_TIER: u8 = 3;

/// Hint verbosity: every refusal names rule, path and fix.
pub fn hint_for(code: &str) -> &'static str {
    match code {
        "missing_intent" => "name 1..=2048 bytes of intent, then re-run",
        "intent_too_long" => "shorten intent to 2048 bytes or split writes",
        "bad_dry_run_id" => "preview via tx.preview, re-run with rcpt_ id",
        "dry_run_id_too_long" => "re-preview for a fresh rcpt_ id",
        "empty_batch" => "send 1..=256 ops in one tx.preview call",
        "too_many_ops" => "split into smaller batches",
        "destructive_tier" => "ask for an elevated session for deletes",
        "hostile_text" => "discard the injected line, then re-run",
        "unknown_tool" => "re-run with a manifest-listed tool name",
        "over_cap" => "shrink payload within caps, then re-run",
        _ => "re-run with legal values",
    }
}

/// Rule path for a refusal code.
pub fn refusal_path(code: &str) -> &'static str {
    match code {
        "missing_intent" | "intent_too_long" => "door.check/intent",
        "bad_dry_run_id" | "dry_run_id_too_long" => "door.check/dry_run",
        "empty_batch" | "too_many_ops" => "door.check/args",
        "destructive_tier" => "door.check/policy",
        "hostile_text" | "unknown_tool" | "over_cap" => "door.check/scan",
        _ => "door.check",
    }
}

/// One-line hint suffix: rule, path and fix.
pub fn hint_line(code: &str) -> String {
    format!(
        "rule={code} path={} hint={}",
        refusal_path(code),
        hint_for(code)
    )
}

#[path = "packs/check_profile.rs"]
pub mod check_profile;
/// Write class of one door verb: what the policy gate demands of it.
///
/// ```
/// use engine_endpoint::guards::{verb_class, VerbClass};
///
/// assert_eq!(verb_class("scene.place"), Some(VerbClass::Mutating));
/// assert_eq!(verb_class("tx.apply"), Some(VerbClass::Destructive));
/// assert_eq!(verb_class("shell.exec"), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerbClass {
    /// State-changing writes that keep every node alive (moves, inserts).
    /// The format gate admits them; the policy gate always passes them.
    Mutating,
    /// Delete-capable writes. The format gate admits them; the policy gate
    /// additionally needs an elevated session.
    Destructive,
}

impl VerbClass {
    /// Short class name for digests and logs.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mutating => "mutating",
            Self::Destructive => "destructive",
        }
    }
}

/// Write class of one door verb by name. `scene.place` mutates,
/// `tx.apply` is the delete-capable path; every other name is `None`, so an
/// unclassified verb fails closed until it is classified here.
pub const fn verb_class(verb: &str) -> Option<VerbClass> {
    // `const` string compare without a dependency: length plus bytes.
    if verb.len() == 11
        && verb.as_bytes()[0] == b's'
        && verb.as_bytes()[1] == b'c'
        && verb.as_bytes()[2] == b'e'
        && verb.as_bytes()[3] == b'n'
        && verb.as_bytes()[4] == b'e'
        && verb.as_bytes()[5] == b'.'
        && verb.as_bytes()[6] == b'p'
        && verb.as_bytes()[7] == b'l'
        && verb.as_bytes()[8] == b'a'
        && verb.as_bytes()[9] == b'c'
        && verb.as_bytes()[10] == b'e'
    {
        return Some(VerbClass::Mutating);
    }
    if verb.len() == 8
        && verb.as_bytes()[0] == b't'
        && verb.as_bytes()[1] == b'x'
        && verb.as_bytes()[2] == b'.'
        && verb.as_bytes()[3] == b'a'
        && verb.as_bytes()[4] == b'p'
        && verb.as_bytes()[5] == b'p'
        && verb.as_bytes()[6] == b'l'
        && verb.as_bytes()[7] == b'y'
    {
        return Some(VerbClass::Destructive);
    }
    None
}

/// Write class of one batch: batches holding a remove are destructive, the
/// rest mutate. The door's tier step maps its remove scan through here so
/// the class decision lives in one place.
pub const fn class_for_batch(has_remove: bool) -> VerbClass {
    if has_remove {
        VerbClass::Destructive
    } else {
        VerbClass::Mutating
    }
}

/// Why one guard check refused a write. Every variant names the legal values
/// plus one recovery, so a weak model retries correctly without a second
/// round trip. Pure data: the document, journal, capability and ledger are
/// untouched by every gate that mints one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardError {
    /// Machine code (`missing_intent`, `bad_dry_run_id`, ...).
    pub code: &'static str,
    /// Detail with the legal values embedded plus one recovery.
    pub message: String,
}

impl GuardError {
    /// New error with a static code.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl core::fmt::Display for GuardError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for GuardError {}

/// `RequireIntent`: every write names a non-empty intent within
/// [`INTENT_MAX_BYTES`] bytes. Empty intents refuse with `missing_intent`,
/// over-long ones with `intent_too_long`; both name the bound plus one
/// recovery.
pub fn check_intent(intent: &str) -> Result<(), GuardError> {
    if intent.is_empty() {
        return Err(GuardError::new(
            "missing_intent",
            format!(
                "write names no intent; submit 1..={INTENT_MAX_BYTES} bytes of intent text \
                 (what the write changes and why), then re-run"
            ),
        ));
    }
    check_intent_len(intent)
}

/// Intent length cap without the non-empty rule, for verbs whose empty
/// intent is a conflict rather than a refusal. Over-long intents refuse with
/// `intent_too_long`; anything else passes.
pub fn check_intent_len(intent: &str) -> Result<(), GuardError> {
    if intent.len() > INTENT_MAX_BYTES {
        return Err(GuardError::new(
            "intent_too_long",
            format!(
                "intent holds {} bytes; at most {INTENT_MAX_BYTES} per write; \
                 shorten the intent or split into smaller writes",
                intent.len()
            ),
        ));
    }
    Ok(())
}

/// `RequireDryRunId`: every mutating write carries the dry-run receipt id
/// that its preview minted. The id rides the `rcpt_` prefix under a 512-byte
/// cap; anything else refuses with a named error plus one recovery. A bad
/// prefix is `bad_dry_run_id`, an over-long id is `dry_run_id_too_long`.
pub fn check_dry_run_id(id: &str) -> Result<(), GuardError> {
    let Some(rest) = id.strip_prefix(DRY_RUN_ID_PREFIX) else {
        return Err(GuardError::new(
            "bad_dry_run_id",
            format!(
                "dry-run id `{id}` does not ride the `{DRY_RUN_ID_PREFIX}` prefix; \
                 preview the batch through tx.preview, then re-run with the minted receipt id"
            ),
        ));
    };
    if rest.is_empty() {
        return Err(GuardError::new(
            "bad_dry_run_id",
            format!(
                "dry-run id `{id}` names no receipt after the `{DRY_RUN_ID_PREFIX}` prefix; \
                 preview the batch through tx.preview, then re-run with the minted receipt id"
            ),
        ));
    }
    if id.len() > DRY_RUN_ID_MAX_BYTES {
        return Err(GuardError::new(
            "dry_run_id_too_long",
            format!(
                "dry-run id holds {} bytes; at most {DRY_RUN_ID_MAX_BYTES} including the \
                 `{DRY_RUN_ID_PREFIX}` prefix; re-preview for a fresh receipt id",
                id.len()
            ),
        ));
    }
    Ok(())
}

/// Args length cap: a write batch holds 1..=[`ARGS_MAX_OPS`] ops. Empty
/// batches refuse with `empty_batch`, oversized ones with `too_many_ops`;
/// both name the bound plus one recovery. The door's shape step delegates
/// here, so previews and applies refuse identically.
pub fn check_args_ops(ops: usize) -> Result<(), GuardError> {
    if ops == 0 {
        return Err(GuardError::new(
            "empty_batch",
            format!("batch holds no ops; send 1..={ARGS_MAX_OPS} ops in one tx.preview call"),
        ));
    }
    if ops > ARGS_MAX_OPS {
        return Err(GuardError::new(
            "too_many_ops",
            format!(
                "batch holds {ops} ops, transport admits 1..={ARGS_MAX_OPS}; \
                 split into smaller batches"
            ),
        ));
    }
    Ok(())
}

/// Policy gate: mutating verbs pass, destructive verbs need an elevated
/// session. A destructive verb at a low tier refuses with
/// `destructive_tier`, naming the tier plus one recovery; anything else
/// passes. Pure: reads only the class and the tier decision.
pub fn check_policy(
    class: VerbClass,
    destructive_allowed: bool,
    tier: u8,
) -> Result<(), GuardError> {
    if class == VerbClass::Destructive && !destructive_allowed {
        return Err(GuardError::new(
            "destructive_tier",
            format!(
                "remove needs tier >= {DESTRUCTIVE_TIER} (this session runs tier {tier}); \
                 run reads and previews at any tier and ask for an elevated \
                 session for deletes"
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(len: usize) -> String {
        let mut id = DRY_RUN_ID_PREFIX.to_string();
        while id.len() < len {
            id.push('r');
        }
        id
    }

    #[test]
    fn guards_format_dry_run_id_prefix_and_cap() {
        // Good receipt admits.
        assert!(check_dry_run_id("rcpt_7f3a").is_ok());
        // Bad prefix refuses with the named error plus a recovery.
        for bad in ["", "7f3a", "RCPT_7f3a", "id_rcpt_7f3a"] {
            let refused = check_dry_run_id(bad).expect_err("bad receipt refuses");
            assert_eq!(refused.code, "bad_dry_run_id");
            assert!(refused.message.contains(DRY_RUN_ID_PREFIX));
            assert!(refused.message.contains("tx.preview"));
        }
        // Bare prefix with no receipt refuses the same way.
        let bare = check_dry_run_id(DRY_RUN_ID_PREFIX).expect_err("bare prefix refuses");
        assert_eq!(bare.code, "bad_dry_run_id");
        // The 512 cap holds exactly, then refuses with its own code.
        assert!(check_dry_run_id(&receipt(DRY_RUN_ID_MAX_BYTES)).is_ok());
        let over = check_dry_run_id(&receipt(DRY_RUN_ID_MAX_BYTES + 1))
            .expect_err("over-long receipt refuses");
        assert_eq!(over.code, "dry_run_id_too_long");
        assert!(over.message.contains("512"));
        // Same inputs twice seal the same verdicts: the gate replays exactly.
        let first = check_dry_run_id("nope");
        let second = check_dry_run_id("nope");
        assert_eq!(first, second, "replay without a model drifts");
    }

    #[test]
    fn guards_format_intent_args_policy_and_classes() {
        // RequireIntent: empty refuses, the cap holds, over-long refuses.
        let missing = check_intent("").expect_err("empty intent refuses");
        assert_eq!(missing.code, "missing_intent");
        assert!(missing.message.contains("2048"));
        assert!(check_intent("move the brazier north").is_ok());
        let long = "w".repeat(INTENT_MAX_BYTES + 1);
        let over = check_intent(&long).expect_err("over-long intent refuses");
        assert_eq!(over.code, "intent_too_long");
        // Length-only cap: empty passes (a conflict, not a refusal), long refuses.
        assert!(check_intent_len("").is_ok());
        assert!(check_intent_len(&long).is_err());
        // Args cap: empty and oversized refuse with the door codes, 1..=256 admits.
        let empty = check_args_ops(0).expect_err("empty batch refuses");
        assert_eq!(empty.code, "empty_batch");
        assert!(empty.message.contains("1..="));
        let many = check_args_ops(ARGS_MAX_OPS + 1).expect_err("oversized batch refuses");
        assert_eq!(many.code, "too_many_ops");
        assert!(many.message.contains("split"));
        assert!(check_args_ops(1).is_ok());
        assert!(check_args_ops(ARGS_MAX_OPS).is_ok());
        // Policy: destructive needs the elevated tier, mutating always passes.
        let tier = check_policy(VerbClass::Destructive, false, 0).expect_err("tier refuses");
        assert_eq!(tier.code, "destructive_tier");
        assert!(tier.message.contains("tier"));
        assert!(check_policy(VerbClass::Destructive, true, DESTRUCTIVE_TIER).is_ok());
        assert!(check_policy(VerbClass::Mutating, false, 0).is_ok());
        // Batch classes: removes are destructive, anything else mutates.
        assert_eq!(class_for_batch(true), VerbClass::Destructive);
        assert_eq!(class_for_batch(false), VerbClass::Mutating);
        // Verb table: the two classified verbs plus fail-closed unknowns.
        let mutating = verb_class("scene.place").expect("scene.place is classified");
        let destructive = verb_class("tx.apply").expect("tx.apply is classified");
        assert_eq!(mutating, VerbClass::Mutating);
        assert_eq!(destructive, VerbClass::Destructive);
        assert_eq!(verb_class("shell.exec"), None);
        // Each single gate replays exactly without a model.
        let replay = [
            check_intent("move the brazier"),
            check_intent("move the brazier"),
        ];
        assert_eq!(replay[0], replay[1], "replay without a model drifts");
    }
}
