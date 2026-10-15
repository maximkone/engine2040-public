//! Shared brief envelope: one typed envelope every AB stage consumes.
//!
//! Owner is `engine-endpoint`; AB1-AB4 and AB9 build through this type
//! instead of per-stage budgets, and over-budget stages refuse with legal
//! values. Units are chars of brief text, seeds and repair rounds. The id
//! scheme is FNV-1a over the text bytes plus the seed, so the same text plus
//! seed names the same brief on every machine. Offline authoring only: never
//! constructed or checked in the tick. Errors are typed [`BriefError`] for
//! bad envelopes and [`BriefRefusal`] for spent budgets.

use engine_core::{hash_bytes, Fnv64};

/// Longest brief text, in Unicode scalar values. Covers the AB1 level bound.
pub const BRIEF_MAX_CHARS: usize = 4096;
/// Repair rounds a level, character, slice or replay stage may spend.
pub const REPAIR_BUDGET_DEFAULT: u8 = 3;
/// Typed repairs an encounter stage may spend before refusing.
pub const REPAIR_BUDGET_ENCOUNTER: u8 = 5;
/// Largest repair budget any stage may carry in one envelope.
pub const REPAIR_BUDGET_MAX: u8 = 5;

/// Chain stage a brief envelope feeds. Tags are stable lowercase words for
/// receipts and digests; AB4 (`Slice`) and AB9 (`Replay`) envelopes already
/// construct so those rows land on this type with no new shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BriefStage {
    /// AB1 brief to level.
    Level,
    /// AB2 brief to character.
    Character,
    /// AB3 brief to encounter.
    Encounter,
    /// AB4 brief to playable slice.
    Slice,
    /// AB9 offline brief replay.
    Replay,
}

impl BriefStage {
    /// Stable tag carried by envelopes, refusals and receipts.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Level => "level",
            Self::Character => "character",
            Self::Encounter => "encounter",
            Self::Slice => "playable-slice",
            Self::Replay => "offline-replay",
        }
    }

    /// Every stage tag in canonical order, for refusal messages.
    pub const fn all_tags() -> &'static [&'static str] {
        &[
            "level",
            "character",
            "encounter",
            "playable-slice",
            "offline-replay",
        ]
    }

    /// Repair budget a fresh envelope of this stage carries: 3 rounds, or 5
    /// typed repairs for encounters.
    pub const fn default_budget(self) -> u8 {
        match self {
            Self::Encounter => REPAIR_BUDGET_ENCOUNTER,
            _ => REPAIR_BUDGET_DEFAULT,
        }
    }

    /// Parse a stage tag; unknown tags refuse with the legal list named.
    pub fn parse(text: &str) -> Result<Self, BriefError> {
        match text {
            "level" => Ok(Self::Level),
            "character" => Ok(Self::Character),
            "encounter" => Ok(Self::Encounter),
            "playable-slice" => Ok(Self::Slice),
            "offline-replay" => Ok(Self::Replay),
            _ => Err(BriefError::BadStage {
                stage: text.to_owned(),
            }),
        }
    }
}

/// What stopped an envelope from being built. Every message names the legal
/// values; nothing is staged or spent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BriefError {
    /// The brief holds no usable text.
    Empty,
    /// The brief holds more than `BRIEF_MAX_CHARS` scalar values.
    TooLong {
        /// Measured length in scalar values.
        len: usize,
    },
    /// The repair budget sits outside `0..=REPAIR_BUDGET_MAX`.
    BadBudget {
        /// Requested budget in repair rounds.
        budget: u8,
    },
    /// The stage tag is unknown.
    BadStage {
        /// Rejected tag text.
        stage: String,
    },
}

impl BriefError {
    /// Machine code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Empty => "empty_brief",
            Self::TooLong { .. } => "brief_too_long",
            Self::BadBudget { .. } => "bad_budget",
            Self::BadStage { .. } => "bad_stage",
        }
    }
}

impl core::fmt::Display for BriefError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(
                f,
                "empty_brief: submit 1..={BRIEF_MAX_CHARS} chars of brief text"
            ),
            Self::TooLong { len } => write!(
                f,
                "brief_too_long: brief holds {len} chars; at most {BRIEF_MAX_CHARS}"
            ),
            Self::BadBudget { budget } => write!(
                f,
                "bad_budget: repair budget {budget} is outside 0..={REPAIR_BUDGET_MAX}; \
                 submit 3 for level, character, slice and replay stages, 5 for encounters"
            ),
            Self::BadStage { stage } => write!(
                f,
                "bad_stage: stage `{stage}` is unknown; submit one of {}",
                BriefStage::all_tags().join(", ")
            ),
        }
    }
}

impl std::error::Error for BriefError {}

/// Shared brief-id scheme: FNV-1a over the text bytes, then the seed in
/// little-endian bytes. This is the one id AB1-AB4 and AB9 stamp; no stage
/// invents its own brief digest.
pub fn brief_id_of(text: &str, seed: u64) -> u64 {
    let mut hasher = Fnv64::with_state(hash_bytes(text.as_bytes()));
    hasher.write(&seed.to_le_bytes());
    hasher.finish()
}

/// One typed envelope every AB stage consumes: brief id, seed, stage, repair
/// budget and the raw text. Field-for-field the AB1 level shape (`brief_id`,
/// `seed`, `stage`, `repair_budget`, `text`); AB3 maps its bounded text plus
/// seed onto it with the 5-typed-repair budget; AB4 and AB9 record and replay
/// it verbatim with providers off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefEnvelope {
    brief_id: u64,
    seed: u64,
    stage: BriefStage,
    repair_budget: u8,
    text: String,
}

impl BriefEnvelope {
    /// Validate a brief into its envelope under the stage default budget (3
    /// rounds, 5 typed repairs for encounters). Empty and oversize briefs
    /// refuse with the legal values; grammar errors surface at stage time.
    pub fn new(text: &str, seed: u64, stage: BriefStage) -> Result<Self, BriefError> {
        Self::with_budget(text, seed, stage, stage.default_budget())
    }

    /// Validate a brief into its envelope under an explicit budget in
    /// `0..=REPAIR_BUDGET_MAX`. Stages pass their own cap here instead of
    /// keeping a private budget type.
    pub fn with_budget(
        text: &str,
        seed: u64,
        stage: BriefStage,
        repair_budget: u8,
    ) -> Result<Self, BriefError> {
        let len = text.chars().count();
        if text.split_whitespace().next().is_none() {
            return Err(BriefError::Empty);
        }
        if len > BRIEF_MAX_CHARS {
            return Err(BriefError::TooLong { len });
        }
        if repair_budget > REPAIR_BUDGET_MAX {
            return Err(BriefError::BadBudget {
                budget: repair_budget,
            });
        }
        Ok(Self {
            brief_id: brief_id_of(text, seed),
            seed,
            stage,
            repair_budget,
            text: text.to_owned(),
        })
    }

    /// FNV over the brief text plus the seed; keys idempotency and replay.
    pub const fn brief_id(&self) -> u64 {
        self.brief_id
    }

    /// Seed this brief expands under; same text plus seed replays identically.
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Chain stage this envelope feeds.
    pub const fn stage(&self) -> BriefStage {
        self.stage
    }

    /// Repair rounds this envelope allows before the stage must refuse.
    pub const fn repair_budget(&self) -> u8 {
        self.repair_budget
    }

    /// Raw brief text the stage parses.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// True while another repair round fits the budget: `repairs_used` counts
    /// consumed rounds, so 0..`repair_budget` pass and `repair_budget` refuses.
    pub const fn budget_allows(&self, repairs_used: u8) -> bool {
        repairs_used < self.repair_budget
    }

    /// Stable one-line record for AB9 replay logs: id, stage, seed, budget.
    /// Live and offline runs stamp the same line for the same envelope.
    pub fn record_line(&self) -> String {
        format!(
            "brief {:016x} stage {} seed {} budget {}",
            self.brief_id,
            self.stage.as_str(),
            self.seed,
            self.repair_budget
        )
    }

    /// Repair gate: `Ok` while `repairs_used` still fits the budget, else the
    /// over-budget refusal carrying the culprit conflicts, the gate findings
    /// and the legal values. Findings and conflicts are cloned only on the
    /// refusal path, so passing rounds pay no allocation for them here.
    pub fn check_repairs(
        &self,
        repairs_used: u8,
        conflicts: &[String],
        findings: &[String],
    ) -> Result<(), BriefRefusal> {
        if self.budget_allows(repairs_used) {
            return Ok(());
        }
        Err(self.refuse_over_budget(repairs_used, conflicts, findings))
    }

    /// Build the over-budget refusal: brief id, stage, consumed budget, the
    /// conflicting lines, the named findings, the reason and the legal values.
    /// Call sites pass the findings that drove the last repair round, so the
    /// refusal always says what failed, never just that it failed.
    pub fn refuse_over_budget(
        &self,
        repairs_used: u8,
        conflicts: &[String],
        findings: &[String],
    ) -> BriefRefusal {
        BriefRefusal {
            brief_id: self.brief_id,
            stage: self.stage,
            repairs_used,
            repair_budget: self.repair_budget,
            conflicts: conflicts.to_vec(),
            findings: findings.to_vec(),
            reason: format!(
                "stage `{}` spent {repairs_used} repair round(s) (budget {}), refusing repair {}; \
                 {} conflict(s), {} finding(s): {}",
                self.stage.as_str(),
                self.repair_budget,
                repairs_used.saturating_add(1),
                conflicts.len(),
                findings.len(),
                findings.join(" | ")
            ),
            legal: BriefRefusal::legal_text(self.stage, self.repair_budget),
        }
    }
}

/// Refusal: the shared shape over-budget stages return instead of chatting
/// forever. Carries the violated budget plus one recovery and the gate
/// findings, so authors can fix the brief; writes nothing by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefRefusal {
    /// Envelope brief id.
    pub brief_id: u64,
    /// Chain stage that refused.
    pub stage: BriefStage,
    /// Repair rounds consumed when the stage refused.
    pub repairs_used: u8,
    /// Budget the envelope allowed.
    pub repair_budget: u8,
    /// Culprit source lines (or the stuck item line).
    pub conflicts: Vec<String>,
    /// Gate finding lines behind the refusal, in gate order.
    pub findings: Vec<String>,
    /// Why the stage refused, with the counts embedded.
    pub reason: String,
    /// Legal values plus one recovery for the next attempt.
    pub legal: String,
}

impl BriefRefusal {
    /// Machine code for this refusal.
    pub fn code(&self) -> &'static str {
        "over_budget"
    }

    /// Legal values for a stage plus budget: the cap and one recovery.
    pub fn legal_text(stage: BriefStage, repair_budget: u8) -> String {
        format!(
            "at most {repair_budget} repair round(s) per brief for stage `{}`; \
             shrink the brief, loosen ranges or spread props, then retry with a fresh envelope",
            stage.as_str()
        )
    }
}

impl core::fmt::Display for BriefRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "over_budget: {}; conflicts: [{}]; findings: [{}]; legal: {}",
            self.reason,
            self.conflicts.join(", "),
            self.findings.join(", "),
            self.legal
        )
    }
}

impl std::error::Error for BriefRefusal {}

#[cfg(test)]
mod tests {
    use super::*;

    fn level_text(name: &str) -> String {
        format!("prop crate {name}\nwant {name} next_to wall:north\nspawn {name}")
    }

    fn envelope(text: &str, seed: u64, stage: BriefStage) -> BriefEnvelope {
        BriefEnvelope::new(text, seed, stage).expect("fixture brief is valid")
    }

    #[test]
    fn twenty_briefs_across_level_and_encounter_share_one_envelope() {
        let level_names = [
            "crate_a", "crate_b", "crate_c", "crate_d", "crate_e", "crate_f", "crate_g", "crate_h",
            "crate_i", "crate_j",
        ];
        let encounter_texts = [
            "a quiet duel at dawn in the training yard",
            "a rivalry grudge match under the noon sun",
            "a brutal cauldron final before the crowd",
            "an advanced duel with footwork drills",
            "a training duel for the new recruits",
            "a pressure rivalry between rival houses",
            "an easy sparring circle for beginners",
            "a hard final between champions",
            "a marathon duel of endurance",
            "an exhausting rivalry across the sand",
        ];
        // One envelope type for both stages: a single mixed vector holds all 20.
        let mut shared: Vec<BriefEnvelope> = Vec::with_capacity(20);
        for (index, name) in level_names.iter().enumerate() {
            shared.push(envelope(
                &level_text(name),
                2040 + index as u64,
                BriefStage::Level,
            ));
        }
        for (index, text) in encounter_texts.iter().enumerate() {
            shared.push(envelope(text, 3040 + index as u64, BriefStage::Encounter));
        }
        assert_eq!(shared.len(), 20);
        let levels: Vec<&BriefEnvelope> = shared
            .iter()
            .filter(|env| env.stage() == BriefStage::Level)
            .collect();
        let encounters: Vec<&BriefEnvelope> = shared
            .iter()
            .filter(|env| env.stage() == BriefStage::Encounter)
            .collect();
        assert_eq!(levels.len(), 10);
        assert_eq!(encounters.len(), 10);
        assert!(
            levels.iter().all(|env| env.repair_budget() == 3),
            "AB1 stages budget 3 repairs"
        );
        assert!(
            encounters.iter().all(|env| env.repair_budget() == 5),
            "AB3 stages budget 5 typed repairs"
        );
        // Every brief names a distinct id.
        let mut ids: Vec<u64> = shared.iter().map(|env| env.brief_id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 20, "distinct briefs name distinct ids");
        // Same text plus seed replays the same envelope (AB9 offline shape).
        let again = envelope(&level_text("crate_a"), 2040, BriefStage::Level);
        assert_eq!(shared[0], again, "same inputs replay identically");
    }

    #[test]
    fn fourth_repair_refused_with_findings_named() {
        let env = envelope(&level_text("crate_q"), 77, BriefStage::Level);
        assert_eq!(env.repair_budget(), 3);
        let conflicts = vec!["want crate_q next_to wall:north".to_owned()];
        let findings = vec![
            "finding overlap 1 2 high 40".to_owned(),
            "conflict unreachable crate_q - yellow 0".to_owned(),
        ];
        // Repairs 1-3 fit the budget.
        for used in [0u8, 1, 2] {
            assert!(
                env.check_repairs(used, &conflicts, &findings).is_ok(),
                "repair {} of 3 passes",
                used + 1
            );
        }
        // The 4th repair refuses with the findings named.
        let refused = env
            .check_repairs(3, &conflicts, &findings)
            .expect_err("4th repair refuses");
        assert_eq!(refused.code(), "over_budget");
        assert_eq!(refused.brief_id, env.brief_id());
        assert_eq!(refused.stage, BriefStage::Level);
        assert_eq!((refused.repairs_used, refused.repair_budget), (3, 3));
        assert_eq!(refused.conflicts, conflicts);
        assert_eq!(refused.findings, findings);
        assert!(
            refused.reason.contains("finding overlap 1 2 high 40"),
            "reason names findings: {}",
            refused.reason
        );
        assert!(
            refused
                .reason
                .contains("conflict unreachable crate_q - yellow 0"),
            "reason names findings: {}",
            refused.reason
        );
        assert!(
            refused.reason.contains("budget 3"),
            "reason: {}",
            refused.reason
        );
        assert!(
            refused.legal.contains("at most 3"),
            "legal: {}",
            refused.legal
        );
        assert!(
            refused.legal.contains("`level`"),
            "legal names the stage: {}",
            refused.legal
        );
        let text = format!("{refused}");
        assert!(text.contains("over_budget"), "display: {text}");
        assert!(text.contains("finding overlap"), "display: {text}");
    }

    #[test]
    fn encounter_stage_allows_five_typed_repairs_then_refuses() {
        let env = envelope(
            "a brutal cauldron final before the crowd",
            9,
            BriefStage::Encounter,
        );
        assert_eq!(env.repair_budget(), 5);
        let conflicts = vec!["fork win_rate 900 permille outside [400, 600]".to_owned()];
        let findings = vec!["fork kill_share 800 permille over the 350 cap".to_owned()];
        for used in [0u8, 1, 2, 3, 4] {
            assert!(
                env.check_repairs(used, &conflicts, &findings).is_ok(),
                "typed repair {} of 5 passes",
                used + 1
            );
        }
        let refused = env
            .check_repairs(5, &conflicts, &findings)
            .expect_err("6th repair refuses");
        assert_eq!((refused.repairs_used, refused.repair_budget), (5, 5));
        assert_eq!(refused.findings, findings);
        assert!(
            refused.legal.contains("at most 5"),
            "legal: {}",
            refused.legal
        );
    }

    #[test]
    fn empty_and_overlong_briefs_refuse_with_legal_values() {
        let empty = BriefEnvelope::new("   \n  ", 1, BriefStage::Level).expect_err("blank refuses");
        assert_eq!(empty.code(), "empty_brief");
        assert!(format!("{empty}").contains("1..=4096"), "legal: {empty}");
        let long = "x ".repeat(BRIEF_MAX_CHARS + 1);
        let over = BriefEnvelope::new(&long, 1, BriefStage::Level).expect_err("overlong refuses");
        assert_eq!(over.code(), "brief_too_long");
        assert!(format!("{over}").contains("4096"), "legal: {over}");
        // Boundary: exactly the cap still lands.
        let edge = "x".repeat(BRIEF_MAX_CHARS);
        assert!(
            BriefEnvelope::new(&edge, 1, BriefStage::Level).is_ok(),
            "exactly 4096 chars lands"
        );
    }

    #[test]
    fn bad_budget_and_bad_stage_refuse_with_legal_values() {
        let over = BriefEnvelope::with_budget("a quiet duel", 1, BriefStage::Level, 6)
            .expect_err("budget 6 refuses");
        assert_eq!(over.code(), "bad_budget");
        assert!(format!("{over}").contains("0..=5"), "legal: {over}");
        let bad = BriefStage::parse("siege").expect_err("unknown stage refuses");
        assert_eq!(bad.code(), "bad_stage");
        let text = format!("{bad}");
        assert!(text.contains("level"), "legal: {text}");
        assert!(text.contains("encounter"), "legal: {text}");
        // Zero budget is legal: the first repair already refuses.
        let strict = BriefEnvelope::with_budget("a quiet duel", 1, BriefStage::Level, 0)
            .expect("zero budget is legal");
        assert!(!strict.budget_allows(0));
    }

    #[test]
    fn brief_ids_are_deterministic_and_seed_sensitive() {
        // Reference vector (FNV-1a over text bytes plus seed LE bytes).
        assert_eq!(brief_id_of("prop crate probe_a", 7), 0xf919fb008dbe0d32);
        let first = brief_id_of("prop crate probe_a", 7);
        let again = brief_id_of("prop crate probe_a", 7);
        assert_eq!(first, again, "same inputs name the same brief");
        assert_ne!(
            first,
            brief_id_of("prop crate probe_a", 8),
            "the seed feeds the id"
        );
        assert_ne!(
            first,
            brief_id_of("prop crate probe_b", 7),
            "the text feeds the id"
        );
    }

    #[test]
    fn stage_tags_and_default_budgets_match_plan() {
        assert_eq!(BriefStage::Level.as_str(), "level");
        assert_eq!(BriefStage::Character.as_str(), "character");
        assert_eq!(BriefStage::Encounter.as_str(), "encounter");
        assert_eq!(BriefStage::Slice.as_str(), "playable-slice");
        assert_eq!(BriefStage::Replay.as_str(), "offline-replay");
        for tag in BriefStage::all_tags() {
            assert_eq!(BriefStage::parse(tag).expect("tag parses").as_str(), *tag);
        }
        assert_eq!(BriefStage::Level.default_budget(), 3);
        assert_eq!(BriefStage::Character.default_budget(), 3);
        assert_eq!(BriefStage::Encounter.default_budget(), 5);
        assert_eq!(BriefStage::Slice.default_budget(), 3);
        assert_eq!(BriefStage::Replay.default_budget(), 3);
    }

    #[test]
    fn refusal_names_stage_budget_findings_and_recovery() {
        let env = envelope("a quiet duel", 5, BriefStage::Slice);
        let refusal = env.refuse_over_budget(
            3,
            &["prop altar altar_north".to_owned()],
            &["finding navConnect 1 - high 90".to_owned()],
        );
        assert_eq!(refusal.stage, BriefStage::Slice);
        assert!(
            refusal.reason.contains("`playable-slice`"),
            "reason: {}",
            refusal.reason
        );
        assert!(
            refusal.reason.contains("budget 3"),
            "reason: {}",
            refusal.reason
        );
        assert!(
            refusal.legal.contains("fresh envelope"),
            "one recovery: {}",
            refusal.legal
        );
        // Slice and replay envelopes already construct for AB4 and AB9.
        let replay = envelope("a quiet duel", 5, BriefStage::Replay);
        assert_eq!(replay.record_line(), replay.record_line());
        assert_ne!(
            env.record_line(),
            replay.record_line(),
            "stage feeds the record"
        );
    }

    #[test]
    fn envelope_clones_replay_identically_for_offline_stages() {
        let env = envelope(&level_text("crate_z"), 4242, BriefStage::Level);
        let line = env.record_line();
        assert!(line.contains("stage level"), "record: {line}");
        assert!(line.contains("budget 3"), "record: {line}");
        assert!(line.contains("seed 4242"), "record: {line}");
        // Recorded inputs replay byte-identically with providers off.
        let again = BriefEnvelope::new(&level_text("crate_z"), 4242, BriefStage::Level)
            .expect("replay lands");
        assert_eq!(env, again);
        assert_eq!(env.brief_id(), brief_id_of(&level_text("crate_z"), 4242));
    }

    #[test]
    fn passing_rounds_pay_no_refusal_and_keep_order() {
        let env = envelope("an easy sparring circle", 11, BriefStage::Character);
        let conflicts = vec!["clip hurricane_kick missing".to_owned()];
        let findings = vec![
            "finding gate_b 3 - low 10".to_owned(),
            "finding gate_a 1 - high 80".to_owned(),
        ];
        assert!(env.check_repairs(0, &conflicts, &findings).is_ok());
        assert!(env.check_repairs(2, &conflicts, &findings).is_ok());
        let refused = env
            .check_repairs(3, &conflicts, &findings)
            .expect_err("character budget is 3");
        // Gate order survives into the refusal verbatim.
        assert_eq!(refused.findings[0], "finding gate_b 3 - low 10");
        assert_eq!(refused.findings[1], "finding gate_a 1 - high 80");
    }
}
