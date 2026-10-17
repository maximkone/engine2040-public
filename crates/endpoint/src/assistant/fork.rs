use super::*;

/// EA1c fork play-test before apply: the checked [`PreviewReceipt`] runs a
/// canonical arena bout on a worker-thread fork, and the fork metrics decide
/// whether EA1d may apply it.
///
/// Consumer: EA1d apply gate (A6 fork proof) reads [`ForkProof`] on a pass
/// and [`ForkBreach`] on a rejection. Data owner stays the editor command
/// service; this stage owns only the canonical fork input (fixed seed, base
/// and match shape), the balance bounds and the verdict types. Units are
/// fixed ticks and damage permille. Determinism scope: the fork id plus the
/// metrics text — the same receipt plus the same probe plus the same window
/// always names the same fork with identical metrics, and the real match is
/// untouched (snapshot-clone isolation in `engine-arena`). Errors are typed
/// [`ForkTestError`]; a fork-budget overrun quarantines the proposal.
///
/// Balance bounds mirror the VF1 reference-bounds intent (`[350, 650]`
/// tournament win-rate caps in `engine-playtest`) as single-bout caps over
/// one fork window. `engine-playtest` stays outside the endpoint dep edge —
/// no new deps or edges per the packet — so the mirror lives here and the
/// comparison runs locally; the orchestrator may promote these to a shared
/// contract later. Tape-swap probes (`ForkProposal::SwapTape`) validate and
/// replay through this API, but endpoint cannot author tape inputs without
/// an `engine-input` edge, so the loop's own probes are morale and freeze
/// only; EA1d inherits the same boundary.
///
/// Canonical fork-test seed: every proposal tests the same bout, so fork
/// ids compare across proposals and replays match exactly.
pub const FORK_TEST_SEED: u64 = 2040;
/// Canonical fork-test fighter count.
pub const FORK_TEST_FIGHTERS: u8 = 2;
/// Canonical fork-test arena half-extent in meters.
pub const FORK_TEST_HALF_EXTENT_M: f32 = 20.0;
/// Canonical fork-test match length in fixed ticks.
pub const FORK_TEST_MATCH_TICKS: u32 = 600;
/// Canonical fork-test window start (match tick).
pub const FORK_TEST_BASE_TICK: u32 = 0;
/// Default fork window in fixed ticks (a full canonical match).
pub const FORK_TEST_TICKS_DEFAULT: u32 = 600;
/// Fork budget: longest window one test may run. Asking for more quarantines
/// the proposal (see [`ForkTestError::BudgetExceeded`]).
pub const FORK_TEST_TICKS_MAX: u32 = 600;
/// Balance bound: most damage (permille) one fighter may deal in the window
/// before the proposal counts as balance-breaking. Calibrated to full-match
/// windows: open-arena brain bouts on the default path end 0-0 (measured 0
/// permille over the canonical 600 ticks, seed 2040), so any dealt damage
/// orders above this cap only arrives with tape-script probes — the
/// FK1-proven decisive signal, which needs an `engine-input` edge the
/// orchestrator has not added, so this branch is forward cover with a
/// synthetic unit test.
pub const FORK_BALANCE_MAX_DAMAGE_PERMILLE: u16 = 100;
/// Balance bound: a kill in fewer than this many fork ticks breaches — the
/// bout must last at least this long before the first knockout. Same forward
/// cover as the damage cap (measured canonical windows never kill).
pub const FORK_BALANCE_MIN_TICKS_TO_KILL: u32 = 450;

/// Replayable fork evidence for the EA1d apply gate: the fork id, the exact
/// fork input (seed, base, window, probe proposals, prefix tape) and the
/// worker-thread metrics. [`ForkProof::replay`] rebuilds the canonical fork
/// and reruns it; the metrics must match exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct ForkProof {
    /// Deterministic fork id over the receipt hash plus the fork input.
    pub fork_id: u64,
    /// Canonical seed the window ran under (today [`FORK_TEST_SEED`]).
    pub seed: u64,
    /// Match tick the window started at (today [`FORK_TEST_BASE_TICK`]).
    pub base_tick: u32,
    /// Window length in fixed ticks.
    pub ticks: u32,
    /// Probe proposals applied in order; empty is the neutral control.
    pub proposals: Vec<ForkProposal>,
    /// Prefix tape the canonical match ran under (today empty: brains fill
    /// every slot, which is the VF1 tournament shape).
    pub tape: FighterTape,
    /// Worker-thread metrics: damage, time-to-kill, stuck.
    pub metrics: ForkMetrics,
}

impl ForkProof {
    /// Rebuild the canonical fork from the stored input and rerun the
    /// window. `Some` with metrics identical to [`ForkProof::metrics`] on a
    /// well-formed proof; `None` when the stored input rejects (a tampered
    /// proof never replays silently).
    pub fn replay(&self) -> Option<ForkMetrics> {
        canonical_fork_metrics(&self.tape, self.base_tick, &self.proposals, self.ticks)
    }
}

/// Balance rejection: the proof that breached plus the single breaching
/// metric. The fork id, the replayable tape and the metric travel together
/// so EA1d (and reviewers) can rerun the exact window.
#[derive(Debug, Clone, PartialEq)]
pub struct ForkBreach {
    /// The breaching run: fork id, replayable input and full metrics.
    pub proof: ForkProof,
    /// Which metric breached: `damage_dealt`, `ticks_to_kill` or
    /// `stuck_ticks`.
    pub metric: &'static str,
    /// Culprit slot for per-fighter metrics; `None` for `ticks_to_kill`.
    pub slot: Option<u8>,
    /// Measured value in metric units (permille for damage, ticks else).
    pub value: u64,
    /// The breached bound in the same units.
    pub bound: u64,
    /// Human detail quoting the metric, value, bound, fork id and window.
    pub reason: String,
}

/// What stopped a fork play-test. A [`ForkTestError::BalanceBreach`] is a
/// normal rejection (the proposer revises); only
/// [`ForkTestError::BudgetExceeded`] quarantines the proposal.
#[derive(Debug, Clone, PartialEq)]
pub enum ForkTestError {
    /// Verify before any proposal; the loop must observe then propose first.
    NoProposal,
    /// Window length zero; submit `1..=FORK_TEST_TICKS_MAX` ticks.
    BadTicks {
        /// Requested window.
        ticks: u32,
    },
    /// Window longer than [`FORK_TEST_TICKS_MAX`]; the proposal is
    /// quarantined and must not proceed to apply.
    BudgetExceeded {
        /// Requested window.
        requested: u32,
        /// Longest window one test may run.
        budget: u32,
    },
    /// A probe proposal the fork refused (unknown fighter, bad morale, bad
    /// tape); the fork ran nothing.
    BadProbe {
        /// Why the probe failed, with the legal values embedded.
        reason: String,
    },
    /// The canonical fork build rejected (defensive: the canonical config
    /// validates, so this fires only on an internal contract break).
    SnapshotFailed {
        /// What rejected the build.
        reason: String,
    },
    /// The proposal fork-tests cleanly but breaks balance: the breach rides
    /// attached (boxed: the proof is the large variant) with its fork id,
    /// replayable tape and metric.
    BalanceBreach(Box<ForkBreach>),
    /// The verify row would push usage past the budget; nothing was recorded.
    OverBudget {
        /// Tokens the verify row needs.
        needed: u32,
        /// Budget of this loop.
        budget: u32,
    },
    /// Transcript already holds `ASSISTANT_TRANSCRIPT_MAX` entries.
    TranscriptFull,
}

impl ForkTestError {
    /// Machine code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoProposal => "no_proposal",
            Self::BadTicks { .. } => "bad_ticks",
            Self::BudgetExceeded { .. } => "fork_budget_exceeded",
            Self::BadProbe { .. } => "bad_probe",
            Self::SnapshotFailed { .. } => "snapshot_failed",
            Self::BalanceBreach(_) => "balance_breach",
            Self::OverBudget { .. } => "over_budget",
            Self::TranscriptFull => "transcript_full",
        }
    }

    /// True when the proposal is quarantined and must not proceed to apply.
    /// Only a fork-budget overrun quarantines; balance breaches and bad
    /// probes are revisable.
    pub fn quarantines_proposal(&self) -> bool {
        matches!(self, Self::BudgetExceeded { .. })
    }
}

impl std::fmt::Display for ForkTestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoProposal => write!(
                f,
                "no_proposal: fork_test needs a proposal first; \
                 call inspect, then propose_mock or propose_custom"
            ),
            Self::BadTicks { ticks } => write!(
                f,
                "bad_ticks: window holds {ticks} ticks; \
                 submit 1..={FORK_TEST_TICKS_MAX} ticks"
            ),
            Self::BudgetExceeded { requested, budget } => write!(
                f,
                "fork_budget_exceeded: window asks {requested} ticks over a {budget}-tick \
                 fork budget; the proposal is quarantined and must not proceed to apply"
            ),
            Self::BadProbe { reason } => write!(f, "bad_probe: {reason}"),
            Self::SnapshotFailed { reason } => write!(f, "snapshot_failed: {reason}"),
            Self::BalanceBreach(breach) => write!(
                f,
                "balance_breach: {}; fork {} replays the window",
                breach.reason, breach.proof.fork_id
            ),
            Self::OverBudget { needed, budget } => write!(
                f,
                "over_budget: verify row needs {needed} tokens over a {budget} budget; \
                 raise the budget (at most {ASSISTANT_BUDGET_MAX}) or start a new loop"
            ),
            Self::TranscriptFull => write!(
                f,
                "transcript_full: transcript holds {ASSISTANT_TRANSCRIPT_MAX} entries; \
                 start a new loop"
            ),
        }
    }
}

impl std::error::Error for ForkTestError {}

/// Canonical fork-test match config: the same bout every proposal tests.
pub(super) fn fork_test_config() -> ArenaConfig {
    ArenaConfig {
        seed: FORK_TEST_SEED,
        ticks: FORK_TEST_MATCH_TICKS,
        fighters: FORK_TEST_FIGHTERS,
        half_extent_m: FORK_TEST_HALF_EXTENT_M,
        table_version: 1,
    }
}

/// Run the canonical fork window: prefix the canonical match to `base_tick`,
/// apply the probe proposals in order, run `ticks` on the fork worker
/// thread. `None` when the canonical build rejects or a probe proposal is
/// invalid — either way the fork ran nothing and the live match is
/// untouched (snapshot-clone isolation: the worker owns its state).
pub(super) fn canonical_fork_metrics(
    tape: &FighterTape,
    base_tick: u32,
    proposals: &[ForkProposal],
    ticks: u32,
) -> Option<ForkMetrics> {
    let config = fork_test_config();
    let (snapshot, _) = engine_arena::run_prefix(&config, tape, base_tick)?;
    let mut fork = Fork::from_snapshot(snapshot, base_tick)?;
    for proposal in proposals {
        fork.apply_proposal(proposal.clone()).ok()?;
    }
    Some(fork.run_n(ticks))
}

/// Deterministic fork id: FNV-1a over the receipt hash plus the full fork
/// input (seed, base, window, fighter count, probe bytes). Integer bytes
/// only, so the same proposal always names the same fork on any machine.
pub(super) fn fork_id(
    action_hash: u64,
    seed: u64,
    base_tick: u32,
    ticks: u32,
    proposals: &[ForkProposal],
) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write(&action_hash.to_le_bytes());
    hasher.write(&seed.to_le_bytes());
    hasher.write(&base_tick.to_le_bytes());
    hasher.write(&ticks.to_le_bytes());
    hasher.write(&[FORK_TEST_FIGHTERS]);
    for proposal in proposals {
        match proposal {
            ForkProposal::SetAggression { fighter, permille } => {
                hasher.write(&[0, *fighter]);
                hasher.write(&permille.to_le_bytes());
            }
            ForkProposal::SetTier { fighter, tier } => {
                hasher.write(&[3, *fighter]);
                hasher.write(&[tier.tier]);
                hasher.write(&tier.hp_permille.to_le_bytes());
                hasher.write(&tier.tell_ticks.to_le_bytes());
                hasher.write(&tier.stagger_permille.to_le_bytes());
                hasher.write(&[tier.counter]);
            }
            ForkProposal::SwapTape { fighter, inputs } => {
                hasher.write(&[1, *fighter]);
                hasher.write(&(inputs.len() as u64).to_le_bytes());
                for input in inputs {
                    hasher.write(&input.tick.to_le_bytes());
                    hasher.write(&[input.fighter.0]);
                    hasher.write(&[input.mx as u8, input.mz as u8]);
                    hasher.write(&input.facing.to_le_bytes());
                    hasher.write(&[input.buttons.0]);
                }
            }
            ForkProposal::FreezeFighter { fighter } => {
                hasher.write(&[2, *fighter]);
            }
        }
    }
    hasher.finish()
}

/// First breaching metric in fixed damage, time-to-kill, stuck order.
/// `None` when the window sits inside every bound.
pub(super) fn check_balance(proof: &ForkProof) -> Option<ForkBreach> {
    for (slot, dealt) in &proof.metrics.damage_dealt {
        if *dealt > FORK_BALANCE_MAX_DAMAGE_PERMILLE {
            let value = u64::from(*dealt);
            let bound = u64::from(FORK_BALANCE_MAX_DAMAGE_PERMILLE);
            return Some(ForkBreach {
                metric: "damage_dealt",
                slot: Some(*slot),
                value,
                bound,
                reason: format!(
                    "damage_dealt slot {slot} dealt {value} permille over the {bound} cap \
                     (fork {}, {ticks} ticks, seed {seed})",
                    proof.fork_id,
                    ticks = proof.ticks,
                    seed = proof.seed,
                ),
                proof: proof.clone(),
            });
        }
    }
    if let Some(kill) = proof.metrics.ticks_to_kill {
        if kill < FORK_BALANCE_MIN_TICKS_TO_KILL {
            let value = u64::from(kill);
            let bound = u64::from(FORK_BALANCE_MIN_TICKS_TO_KILL);
            return Some(ForkBreach {
                metric: "ticks_to_kill",
                slot: None,
                value,
                bound,
                reason: format!(
                    "ticks_to_kill killed in {value} ticks, faster than the {bound}-tick floor \
                     (fork {}, {ticks} ticks, seed {seed})",
                    proof.fork_id,
                    ticks = proof.ticks,
                    seed = proof.seed,
                ),
                proof: proof.clone(),
            });
        }
    }
    // Stuck out the whole window: the fighter never contested the bout, so
    // the window is degenerate and balance-breaking. Window-relative
    // (measured: the neutral 600-tick control peaks at 163 stuck ticks while
    // a frozen foe reads the full 600; the neutral 120-tick control peaks at
    // 103 while a frozen foe reads 120). Single-tick windows skip the rule:
    // one tick is no bout to judge.
    if proof.ticks > 1 {
        for (slot, stuck) in &proof.metrics.stuck_ticks {
            if *stuck >= proof.ticks {
                let value = u64::from(*stuck);
                let bound = u64::from(proof.ticks);
                return Some(ForkBreach {
                    metric: "stuck_ticks",
                    slot: Some(*slot),
                    value,
                    bound,
                    reason: format!(
                        "stuck_ticks slot {slot} stuck {value} of {bound} window ticks \
                         (fork {}, seed {seed})",
                        proof.fork_id,
                        seed = proof.seed,
                    ),
                    proof: proof.clone(),
                });
            }
        }
    }
    None
}

/// Deterministic verify-row text for a proof: fork id, window, the three
/// metrics and the outcome (pass, or the breaching metric name).
pub(super) fn verify_text(proof: &ForkProof, breach: Option<&ForkBreach>) -> String {
    let damage: Vec<String> = proof
        .metrics
        .damage_dealt
        .iter()
        .map(|(slot, dealt)| format!("{slot}:{dealt}"))
        .collect();
    let stuck: Vec<String> = proof
        .metrics
        .stuck_ticks
        .iter()
        .map(|(slot, ticks)| format!("{slot}:{ticks}"))
        .collect();
    let kill = match proof.metrics.ticks_to_kill {
        Some(ticks) => ticks.to_string(),
        None => "none".to_owned(),
    };
    let outcome = match breach {
        Some(hit) => format!("reject:{}", hit.metric),
        None => "pass".to_owned(),
    };
    format!(
        "verify fork{} ticks{} damage[{}] ttk{} stuck[{}] {outcome}",
        proof.fork_id,
        proof.ticks,
        damage.join(","),
        kill,
        stuck.join(",")
    )
}
