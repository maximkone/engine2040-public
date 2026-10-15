//! Witnessed-plan gate plus evidence receipts: every mutating door proposal
//! prepares against one observed state digest, revalidates authority and
//! conflicts at commit, applies idempotently and keeps an evidence receipt.
//!
//! Consumer: the MCP door mutating verbs (`tx.apply`, `scene.place` and the
//! pack writers that follow). The door observes the document digest plus
//! revision, prepares a [`Proposal`] through [`prepare`], then commits through
//! [`WitnessGate::commit`]. Data owner stays the editor command service; this
//! module owns only the pure gate: the witness binding, the revalidation
//! order and the receipt bytes. Units: digests are FNV-1a 64, revisions are
//! journal counts, tiers are session levels. Budget: one small `String` per
//! refusal, one receipt per commit key; the gate runs at the door, outside
//! the fixed tick. Determinism: FNV-1a plus little-endian words only, no
//! wall clock, no floats; same observe plus prepare plus commit order seals
//! the same receipts on every machine. Errors are typed [`WitnessError`];
//! refusals name the legal values plus one recovery and write nothing.
//!
//! PORT (ideas reimplemented, zero bytes vendored):
//! `Dicklesworthstone/dwarf_fortress_mcp` observe-intent-witness-commit-
//! evidence loop (MIT, shape only, never a dependency): observe the state,
//! name the intent, bind the plan to the witnessed digest, revalidate at
//! commit, retain the pre/post evidence receipt.

use std::collections::BTreeMap;

use engine_core::{hash::FNV64_OFFSET, Fnv64};

use super::guards::{check_intent, DESTRUCTIVE_TIER};

/// Seed domain for witness digests (kept apart from play and replay digests).
pub const WITNESS_DOMAIN: u64 = 0xd02f_1177_1ce0_0029;
/// Longest goal text in bytes; goals stay one line a weak model can quote.
pub const GOAL_MAX_BYTES: usize = 512;
/// Session tier that may land destructive proposals (mirrors the door tier).
pub const WITNESS_DESTRUCTIVE_TIER: u8 = DESTRUCTIVE_TIER;

/// Observed document state one proposal binds to. The digest names the
/// content, the revision names the journal position; both must still match
/// at commit or the commit refuses as stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Witness {
    /// FNV over the observed document content.
    pub digest: u64,
    /// Journal revision of the observed content.
    pub revision: u64,
}

impl Witness {
    /// Observed state at one journal position.
    pub fn new(digest: u64, revision: u64) -> Self {
        Self { digest, revision }
    }
}

/// A mutating proposal bound to one [`Witness`]. The commit key converges
/// the witness, the intent, the goal and the payload, so the same inputs
/// always name the same key and a double commit lands once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// Witnessed digest this proposal prepared against.
    pub witness_digest: u64,
    /// Witnessed revision this proposal prepared against.
    pub witness_revision: u64,
    /// What the write changes and why (1..=2048 bytes, guard-checked).
    pub intent: String,
    /// Goal the evidence receipt must prove (1..=512 bytes).
    pub goal: String,
    /// FNV over the canonical payload bytes the caller staged.
    pub payload_digest: u64,
    /// Converged idempotency key over witness plus intent plus goal plus payload.
    pub commit_key: u64,
    /// Session tier the commit must carry (`0` reads, `3` deletes).
    pub requires_tier: u8,
}

impl Proposal {
    /// Idempotency key of this proposal; a second commit with the same key
    /// answers with the retained receipt instead of re-applying.
    pub const fn commit_key(&self) -> u64 {
        self.commit_key
    }
}

/// Why a prepare or commit refused. Every message names the legal values
/// plus one recovery so a weak model retries without a second round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessError {
    /// Machine code (`missing_intent`, `stale_witness`, ...).
    pub code: &'static str,
    /// Detail with the legal values embedded plus one recovery.
    pub message: String,
}

impl WitnessError {
    /// New error with a static code.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl core::fmt::Display for WitnessError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for WitnessError {}

/// Fold one u64 word into a running FNV-1a 64 digest.
fn mix_word(hash: &mut u64, word: u64) {
    let mut lane = Fnv64::with_state(*hash);
    lane.write(&word.to_le_bytes());
    *hash = lane.finish();
}

/// Converged commit key over witness, intent, goal and payload. Same inputs
/// name the same key on every machine; the absolute value is never compared
/// across shapes.
fn converge_key(
    witness_digest: u64,
    witness_revision: u64,
    intent: &str,
    goal: &str,
    payload_digest: u64,
) -> u64 {
    let mut hash = FNV64_OFFSET ^ WITNESS_DOMAIN;
    mix_word(&mut hash, witness_digest);
    mix_word(&mut hash, witness_revision);
    mix_word(&mut hash, payload_digest);
    let mut intent_lane = Fnv64::with_state(hash);
    intent_lane.write(intent.as_bytes());
    hash = intent_lane.finish();
    let mut goal_lane = Fnv64::with_state(hash);
    goal_lane.write(goal.as_bytes());
    goal_lane.finish()
}

/// Evidence of one landed (or already-landed) commit: the pre/post digests
/// plus the goal proof. Encodes byte-identically; two runs over the same
/// observe plus prepare plus commit order hold the same bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceReceipt {
    /// Document digest before the commit.
    pub pre_digest: u64,
    /// Journal revision before the commit.
    pub pre_revision: u64,
    /// Document digest after the commit.
    pub post_digest: u64,
    /// Journal revision after the commit.
    pub post_revision: u64,
    /// FNV proof that the stated goal drove the post digest.
    pub goal_proof: u64,
    /// Idempotency key of the proposal that landed.
    pub commit_key: u64,
}

impl EvidenceReceipt {
    /// Canonical bytes: domain tag plus six little-endian words. Same
    /// receipt gives the same bytes twice on every machine.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 * 7);
        out.extend_from_slice(b"WITN0001");
        out.extend_from_slice(&self.pre_digest.to_le_bytes());
        out.extend_from_slice(&self.pre_revision.to_le_bytes());
        out.extend_from_slice(&self.post_digest.to_le_bytes());
        out.extend_from_slice(&self.post_revision.to_le_bytes());
        out.extend_from_slice(&self.goal_proof.to_le_bytes());
        out.extend_from_slice(&self.commit_key.to_le_bytes());
        out
    }

    /// Converged digest of the canonical bytes. Equality across runs is the
    /// replay proof; the absolute value is never compared across shapes.
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::with_state(FNV64_OFFSET ^ WITNESS_DOMAIN);
        hasher.write(&self.encode());
        hasher.finish()
    }

    /// One-line receipt a log tail can quote: pre/post digests, revisions,
    /// goal proof and commit key.
    pub fn summary(&self) -> String {
        format!(
            "witness {:016x}@{} -> {:016x}@{} goal {:016x} key {:016x}",
            self.pre_digest,
            self.pre_revision,
            self.post_digest,
            self.post_revision,
            self.goal_proof,
            self.commit_key,
        )
    }
}

/// Prepare one proposal against a witnessed state digest. Validates the
/// intent through the shared guard (empty refuses `missing_intent`,
/// over-long refuses `intent_too_long`), requires a 1..=512 byte goal, and
/// converges the idempotency key; writes nothing.
pub fn prepare(
    witness: Witness,
    intent: &str,
    goal: &str,
    payload_digest: u64,
    requires_tier: u8,
) -> Result<Proposal, WitnessError> {
    if let Err(guard) = check_intent(intent) {
        return Err(WitnessError::new(guard.code, guard.message));
    }
    if goal.is_empty() {
        return Err(WitnessError::new(
            "missing_goal",
            format!(
                "proposal names no goal; submit 1..={GOAL_MAX_BYTES} bytes of goal text \
                 (what done looks like), then re-prepare"
            ),
        ));
    }
    if goal.len() > GOAL_MAX_BYTES {
        return Err(WitnessError::new(
            "goal_too_long",
            format!(
                "goal holds {} bytes; at most {GOAL_MAX_BYTES} per proposal; \
                 shorten the goal or split the work",
                goal.len()
            ),
        ));
    }
    Ok(Proposal {
        witness_digest: witness.digest,
        witness_revision: witness.revision,
        intent: intent.to_owned(),
        goal: goal.to_owned(),
        payload_digest,
        commit_key: converge_key(
            witness.digest,
            witness.revision,
            intent,
            goal,
            payload_digest,
        ),
        requires_tier,
    })
}

/// The commit gate: holds the live document digest plus revision and every
/// landed receipt by commit key. A retried key answers at once with the
/// retained receipt; fresh keys revalidate in one fixed order — witness
/// freshness, then authority, then conflicts — and land exactly once.
#[derive(Debug, Default)]
pub struct WitnessGate {
    /// Live document digest the next commit must still witness.
    current_digest: u64,
    /// Live journal revision the next commit must still witness.
    current_revision: u64,
    /// Landed receipts by commit key, in key order.
    landed: BTreeMap<u64, EvidenceReceipt>,
    /// Commits that changed state (retries never increment this).
    apply_count: u64,
}

impl WitnessGate {
    /// New gate over one observed state; tests and the door seed this from
    /// the same digest plus revision they hand to [`prepare`].
    pub fn new(digest: u64, revision: u64) -> Self {
        Self {
            current_digest: digest,
            current_revision: revision,
            landed: BTreeMap::new(),
            apply_count: 0,
        }
    }

    /// Live document digest.
    pub const fn digest(&self) -> u64 {
        self.current_digest
    }

    /// Live journal revision.
    pub const fn revision(&self) -> u64 {
        self.current_revision
    }

    /// Commits that changed state; a double commit still reads `1`.
    pub const fn apply_count(&self) -> u64 {
        self.apply_count
    }

    /// Retained receipt for one commit key, if that key already landed.
    pub fn retained(&self, commit_key: u64) -> Option<&EvidenceReceipt> {
        self.landed.get(&commit_key)
    }

    /// Commit one proposal: a retried key answers at once with the
    /// retained receipt (the key converges the witness, so only its own
    /// first commit could have landed it); fresh keys revalidate witness
    /// freshness (`stale_witness`), then authority (`authority_refused`),
    /// then conflicts (`has_conflicts`), and land exactly once with the
    /// evidence receipt retained (pre/post digests plus goal proof).
    pub fn commit(
        &mut self,
        proposal: &Proposal,
        authority_tier: u8,
        conflicts: &[&str],
    ) -> Result<EvidenceReceipt, WitnessError> {
        if let Some(kept) = self.landed.get(&proposal.commit_key) {
            return Ok(kept.clone());
        }
        if proposal.witness_digest != self.current_digest
            || proposal.witness_revision != self.current_revision
        {
            return Err(WitnessError::new(
                "stale_witness",
                format!(
                    "proposal witnessed {:016x}@{} but the document stands at {:016x}@{}; \
                     re-observe the state digest plus revision, re-prepare, then re-commit",
                    proposal.witness_digest,
                    proposal.witness_revision,
                    self.current_digest,
                    self.current_revision,
                ),
            ));
        }
        if authority_tier < proposal.requires_tier {
            return Err(WitnessError::new(
                "authority_refused",
                format!(
                    "proposal needs tier >= {} (this session carries tier {authority_tier}); \
                     run reads and previews at any tier and ask for an elevated session",
                    proposal.requires_tier,
                ),
            ));
        }
        if let Some(first) = conflicts.first() {
            return Err(WitnessError::new(
                "has_conflicts",
                format!(
                    "proposal conflicts with `{first}` ({} live conflict(s)); \
                     resolve the conflicts or re-observe, then re-prepare",
                    conflicts.len(),
                ),
            ));
        }
        let pre_digest = self.current_digest;
        let pre_revision = self.current_revision;
        let mut post = FNV64_OFFSET ^ (WITNESS_DOMAIN ^ 0x9c04);
        mix_word(&mut post, pre_digest);
        mix_word(&mut post, proposal.payload_digest);
        mix_word(&mut post, proposal.commit_key);
        let mut proof_lane = Fnv64::with_state(post);
        proof_lane.write(proposal.goal.as_bytes());
        let goal_proof = proof_lane.finish();
        let post_revision = pre_revision.saturating_add(1);
        let receipt = EvidenceReceipt {
            pre_digest,
            pre_revision,
            post_digest: post,
            post_revision,
            goal_proof,
            commit_key: proposal.commit_key,
        };
        self.current_digest = post;
        self.current_revision = post_revision;
        self.landed.insert(proposal.commit_key, receipt.clone());
        self.apply_count = self.apply_count.saturating_add(1);
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: u64 = 0x1111_2222_3333_4444;
    const PAYLOAD: u64 = 0xaaaa_bbbb_cccc_dddd;

    fn witnessed() -> (Witness, WitnessGate) {
        (Witness::new(DIGEST, 7), WitnessGate::new(DIGEST, 7))
    }

    fn planned(witness: Witness) -> Proposal {
        prepare(
            witness,
            "move the brazier north",
            "brazier by the north wall",
            PAYLOAD,
            0,
        )
        .expect("fixture proposal prepares")
    }

    #[test]
    fn witness_plan_stale_refused_with_named_error() {
        let (witness, mut gate) = witnessed();
        let proposal = planned(witness);
        // The document moved after the proposal prepared: stale refuses.
        gate.current_digest = 0xdead_beef_dead_beef;
        let refused = gate
            .commit(&proposal, 0, &[])
            .expect_err("stale witness refuses");
        assert_eq!(refused.code, "stale_witness");
        assert!(refused.message.contains("re-observe"));
        assert_eq!(gate.apply_count(), 0);
        // Nothing landed, so no receipt is retained for the key.
        assert!(gate.retained(proposal.commit_key()).is_none());
        // A fresh observe plus prepare lands on the moved state.
        let fresh = Witness::new(gate.digest(), gate.revision());
        let retry = planned(fresh);
        let receipt = gate.commit(&retry, 0, &[]).expect("fresh witness lands");
        assert_eq!(receipt.pre_digest, 0xdead_beef_dead_beef);
        assert_eq!(gate.apply_count(), 1);
    }

    #[test]
    fn witness_plan_double_commit_applies_once() {
        let (witness, mut gate) = witnessed();
        let proposal = planned(witness);
        let first = gate.commit(&proposal, 0, &[]).expect("first commit lands");
        let second = gate.commit(&proposal, 0, &[]).expect("retry answers");
        // Idempotent: the same receipt twice, one state change.
        assert_eq!(first, second, "double commit must answer identically");
        assert_eq!(first.encode(), second.encode());
        assert_eq!(first.digest(), second.digest());
        assert_eq!(gate.apply_count(), 1);
        assert_eq!(gate.retained(proposal.commit_key()), Some(&first));
        // The receipt carries the evidence: pre/post plus goal proof.
        assert_eq!(first.pre_digest, DIGEST);
        assert_eq!(first.pre_revision, 7);
        assert_eq!(first.post_revision, 8);
        assert_ne!(first.post_digest, first.pre_digest);
        assert_ne!(first.goal_proof, 0);
    }

    #[test]
    fn witness_plan_replay_identical_receipts() {
        // Run one: observe, prepare, commit.
        let (witness, mut gate) = witnessed();
        let proposal = planned(witness);
        let live = gate.commit(&proposal, 0, &[]).expect("live commit lands");
        // Run two: the same order from the same start seals the same bytes.
        let (again_witness, mut again) = witnessed();
        let again_proposal = planned(again_witness);
        assert_eq!(proposal, again_proposal, "prepare must replay exactly");
        let replay = again.commit(&again_proposal, 0, &[]).expect("replay lands");
        assert_eq!(live.encode(), replay.encode(), "receipts replay x2");
        assert_eq!(live.digest(), replay.digest());
        assert_eq!(live.summary(), replay.summary());
        assert!(live.summary().contains("goal"));
    }

    #[test]
    fn witness_plan_authority_conflicts_and_goal_gates() {
        let (witness, mut gate) = witnessed();
        // Destructive proposals need the elevated tier.
        let hard = prepare(
            witness,
            "drop the pillar",
            "pillar gone",
            PAYLOAD,
            WITNESS_DESTRUCTIVE_TIER,
        )
        .expect("destructive proposal prepares");
        let tier = gate
            .commit(&hard, 0, &[])
            .expect_err("low tier refuses deletes");
        assert_eq!(tier.code, "authority_refused");
        assert!(tier.message.contains("tier"));
        // Live conflicts refuse before anything is written.
        let plain = planned(witness);
        let conflict = gate
            .commit(&plain, 0, &["pillar moved by another session"])
            .expect_err("conflicts refuse");
        assert_eq!(conflict.code, "has_conflicts");
        assert!(conflict.message.contains("pillar moved"));
        assert_eq!(gate.apply_count(), 0);
        // Empty and over-long goals refuse at prepare with named errors.
        let missing = prepare(witness, "move it", "", PAYLOAD, 0).expect_err("goal required");
        assert_eq!(missing.code, "missing_goal");
        let long = "g".repeat(GOAL_MAX_BYTES + 1);
        let over = prepare(witness, "move it", &long, PAYLOAD, 0).expect_err("goal cap holds");
        assert_eq!(over.code, "goal_too_long");
        // The guarded intent rules still hold through the witness prepare.
        let intent = prepare(witness, "", "pillar gone", PAYLOAD, 0).expect_err("intent required");
        assert_eq!(intent.code, "missing_intent");
    }
}
