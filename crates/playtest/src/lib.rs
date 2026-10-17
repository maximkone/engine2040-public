//! playtest verifiers as content compilers (AI-NATIVE pillar 5).
//!
//! Consumer: the generation loop (text brief to arena variant), the
//! frame-grounded checks and the A6 fork proof. One call — [`verify`] — runs
//! all three verifiers over an [`ArenaVariant`] and returns a [`Verdict`].
//!
//! Data owner: the caller owns the variant. Input is plain actor/zone data
//! (spawns, gates, props with sizes and costs); this crate never imports
//! editor, scene or command types. The arena fork verifier consumes only the
//! public [`engine_arena`] API (`run_prefix`, `fork`), never its internals.
//!
//! Units: meters in, integer centimeters inside reachability, fixed ticks and
//! permille (`0..=1000`) for win rates. Budgets count draws, triangles and
//! bytes as integers with saturating sums.
//!
//! Determinism scope: same-machine reproducibility. Reachability runs on an
//! integer grid with sorted iteration, so any input order gives the same
//! failures. Win rates come from [`engine_arena`] forks on the default
//! kinematic path: the same variant twice gives the same verdict. No
//! wall-clock time, thread timing or hash-map order enters a verdict.
//!
//! Determinism strategy (no arena `sim`): the arena proves the same fork twice
//! gives identical metrics on the default path, and the default checkpoints
//! already cover brains, combat and locomotion. The opt-in `sim` feature
//! would pull Rapier pipeline state (not covered by public snapshots) and
//! `engine-physics` into the default graph for no verifier benefit, so this
//! crate deliberately does not enable it. Cross-machine float bit-identity
//! stays out of scope per `HASH-LAW.md`.
//!
//! Error policy: every invalid input becomes a [`Failure`] with a reason,
//! never a panic. `pass` is true exactly when `failures` is empty. A run that
//! executed zero checks is never a pass: it adds one `NoChecks` failure.

pub mod agent;
pub mod bench_mirror;
pub mod bot_harness;
pub mod brief_eval;
pub mod budgets;
pub mod causal;
pub mod charter;
pub mod divergence;
mod door;
pub mod eval;
pub mod exploit;
pub mod frame_check;
pub mod keypoint;
pub mod living;
pub mod mirror;
pub mod mirror_craft;
pub mod mirror_gdb;
pub mod mirror_kit;
pub mod mirror_logic;
/// Shared parse errors and tokenizer core for the fixture grammars.
pub mod parse;
pub mod probe_sync;
pub mod rating;
pub mod reachability;
pub mod replay;
pub mod scenario_shape;
pub mod seeded_bug;
pub mod spatial_eval;
pub mod spatial_shapes;
pub mod tasks;
pub mod tournament;
pub mod trigger;
pub mod winrates;

/// Exploit-sweep gate (playtest side): the proof harness resolves the sweep
/// by [`EXPLOIT_GATE_NAME`](exploit::EXPLOIT_GATE_NAME) and calls
/// [`exploit_gate`] through the crate root, mirroring the tournament gate
/// registration. The move-intake and rating consumers read the gate
/// outcome from here.
pub use exploit::{exploit_gate, ExploitGateOutcome, EXPLOIT_GATE_NAME};
/// Living-world eval gate (playtest side): the proof harness resolves
/// the eval by [`LIVING_GATE_NAME`](living::LIVING_GATE_NAME) and calls
/// [`living_gate`] through the crate root, mirroring the tournament gate
/// registration.
pub use living::{living_gate, LivingGateOutcome, LIVING_GATE_NAME};
/// Proof-gate registration (playtest side): the proof harness resolves
/// the balance tournament by [`GATE_NAME`](tournament::GATE_NAME) and calls
/// [`proof_gate`] through the crate root. The registration lives here so a
/// removed or renamed entry fails the build on the proof side instead of
/// silently unwiring the gate.
pub use tournament::{proof_gate, GateOutcome, GATE_NAME};

/// Fixture aggregate for mirror rows: 2 bouts, hold 2, delay 4.
pub fn bot_harness_fixture() -> bot_harness::HarnessAggregate {
    let harness = bot_harness::BotHarness {
        brain: bot_harness::Brain::Seeded,
        hold_ticks: 2,
        delay_frames: 4,
        bouts: 2,
        ticks_per_bout: 60,
        base_seed: 2040,
    };
    harness.run().unwrap_or_default()
}

/// One blocking or decorative prop in meters. Sizes are full extents.
/// `physical` props block movement in reachability; all props cost budget.
#[derive(Clone, PartialEq, Debug)]
pub struct Prop {
    /// Stable id, unique within the variant.
    pub id: u32,
    /// Display name, for reasons only.
    pub name: String,
    /// Center in meters.
    pub x: f32,
    /// Center height in meters.
    pub y: f32,
    /// Center in meters.
    pub z: f32,
    /// Full-extent size in meters, each strictly positive.
    pub sx: f32,
    /// Full-extent size in meters, each strictly positive.
    pub sy: f32,
    /// Full-extent size in meters, each strictly positive.
    pub sz: f32,
    /// True when fighters cannot walk through this prop.
    pub physical: bool,
    /// Estimated triangles for this prop.
    pub triangles: u32,
    /// Estimated resident bytes for this prop.
    pub memory_bytes: u32,
}

/// One fighter spawn point in meters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Spawn {
    /// Stable id, unique within the variant.
    pub id: u32,
    /// Spawn point in meters.
    pub x: f32,
    /// Spawn point in meters.
    pub z: f32,
}

/// One gate (objective/exit) in meters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Gate {
    /// Stable id, unique within the variant.
    pub id: u32,
    /// Gate point in meters.
    pub x: f32,
    /// Gate point in meters.
    pub z: f32,
}

/// Draw/triangle/memory caps for [`budgets`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BudgetCaps {
    /// Maximum draws (one per prop).
    pub max_draws: u32,
    /// Maximum total triangles.
    pub max_triangles: u64,
    /// Maximum total resident bytes.
    pub max_memory_bytes: u64,
}

/// Fork-tournament configuration for [`winrates`].
///
/// The tournament plays `matches` AI-vs-AI scripted-policy matches (the
/// built-in utility brains, one per fighter, with per-fighter morale from
/// [`ArenaVariant::morale` applied as fork proposals). Match `i` uses seed
/// `base_seed.wrapping_add(i)`, so the seed count is exactly `matches` and
/// the same variant always replays the same seeds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BalanceConfig {
    /// Matches per tournament, `1..=winrates::MAX_MATCHES`.
    pub matches: u32,
    /// Fixed ticks per match (`1..=engine_arena::MAX_TICKS`).
    pub match_ticks: u32,
    /// First seed; match `i` uses `base_seed.wrapping_add(i)`.
    pub base_seed: u64,
    /// Minimum acceptable fighter-0 win rate in permille.
    pub min_win_permille: u16,
    /// Maximum acceptable fighter-0 win rate in permille.
    pub max_win_permille: u16,
}

/// One arena variant under test: plain scene data plus verifier configs.
#[derive(Clone, PartialEq, Debug)]
pub struct ArenaVariant {
    /// Variant name, for reasons only.
    pub name: String,
    /// Arena half-extent in meters. Forks need `5.0..=100.0` (the
    /// [`engine_arena`] range); reachability needs finite and positive.
    pub half_extent_m: f32,
    /// Fighter spawns; at least one.
    pub spawns: Vec<Spawn>,
    /// Gates every spawn must reach; at least one.
    pub gates: Vec<Gate>,
    /// Scene props: blockers for reachability, costs for budgets.
    pub props: Vec<Prop>,
    /// Per-fighter brain morale in permille (`0..=1000`, 500 neutral).
    /// Length sets the fighter count (`2..=engine_arena::MAX_PARTICIPANTS`).
    pub morale: Vec<u16>,
    /// Fork-tournament config.
    pub balance: BalanceConfig,
    /// Budget caps.
    pub budgets: BudgetCaps,
}

/// One verifier rejection: which verifier and why.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Failure {
    /// Verifier name (`reachability::VERIFIER`, `winrates::VERIFIER` or
    /// `budgets::VERIFIER`).
    pub verifier: &'static str,
    /// Deterministic human- and machine-readable reason.
    pub reason: String,
}

/// Verifier name for the zero-checks rule: a run that executed no checks is
/// never a pass.
pub const NO_CHECKS: &str = "NoChecks";

/// Compiler verdict over a variant: `pass` is true exactly when `failures`
/// is empty. Verifiers run in fixed order (reachability, win rates,
/// budgets); failures within a verifier are id-sorted. `checks_run` counts
/// the verifier checks that executed, so a run with zero checks is never a
/// pass ([`NO_CHECKS`]).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Verdict {
    /// True when every verifier passed and at least one check ran.
    pub pass: bool,
    /// All rejections, in fixed verifier order.
    pub failures: Vec<Failure>,
    /// Checks that executed: one per verifier family that returned
    /// (reachability, win rates, budgets).
    pub checks_run: u32,
}

impl Verdict {
    /// Combine `failures` with the count of checks that ran. Zero checks adds
    /// one [`NO_CHECKS`] failure, so a hollow run cannot read as a pass.
    pub(crate) fn from_checks(mut failures: Vec<Failure>, checks_run: u32) -> Self {
        if checks_run == 0 {
            failures.push(Failure {
                verifier: NO_CHECKS,
                reason: "no checks ran: zero verifier checks executed, nothing was verified"
                    .to_string(),
            });
        }
        Self {
            pass: failures.is_empty(),
            failures,
            checks_run,
        }
    }
}

/// One printable line: `PASS|FAIL checks_run=N failures=M`.
impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let word = if self.pass { "PASS" } else { "FAIL" };
        write!(
            f,
            "{word} checks_run={} failures={}",
            self.checks_run,
            self.failures.len()
        )
    }
}

/// Run every verifier over `variant` and return the combined verdict.
/// Pure and deterministic: the same variant twice gives the same verdict.
pub fn verify(variant: &ArenaVariant) -> Verdict {
    let mut failures = Vec::new();
    let mut checks_run = 0;
    failures.extend(reachability::check(variant));
    checks_run += 1;
    failures.extend(winrates::check(variant));
    checks_run += 1;
    failures.extend(budgets::check(variant));
    checks_run += 1;
    Verdict::from_checks(failures, checks_run)
}

/// The reference arena every verifier must pass: an open 20 m floor, two
/// spawns, two gates, four small corner braziers, symmetric morale and
/// budgets with headroom. Win-rate bounds `[350, 650]` around the measured
/// symmetric rate (see `winrates` docs); the LG1 loop uses this as its
/// known-good starting variant.
pub fn reference_arena() -> ArenaVariant {
    ArenaVariant {
        name: "reference".to_string(),
        half_extent_m: 20.0,
        spawns: vec![
            Spawn {
                id: 0,
                x: -5.0,
                z: 0.0,
            },
            Spawn {
                id: 1,
                x: 5.0,
                z: 0.0,
            },
        ],
        gates: vec![
            Gate {
                id: 10,
                x: 0.0,
                z: -9.0,
            },
            Gate {
                id: 11,
                x: 0.0,
                z: 9.0,
            },
        ],
        props: vec![
            Prop {
                id: 20,
                name: "brazier_nw".to_string(),
                x: -8.0,
                y: 0.5,
                z: -8.0,
                sx: 1.0,
                sy: 1.0,
                sz: 1.0,
                physical: true,
                triangles: 1200,
                memory_bytes: 48_000,
            },
            Prop {
                id: 21,
                name: "brazier_ne".to_string(),
                x: 8.0,
                y: 0.5,
                z: -8.0,
                sx: 1.0,
                sy: 1.0,
                sz: 1.0,
                physical: true,
                triangles: 1200,
                memory_bytes: 48_000,
            },
            Prop {
                id: 22,
                name: "brazier_sw".to_string(),
                x: -8.0,
                y: 0.5,
                z: 8.0,
                sx: 1.0,
                sy: 1.0,
                sz: 1.0,
                physical: true,
                triangles: 1200,
                memory_bytes: 48_000,
            },
            Prop {
                id: 23,
                name: "brazier_se".to_string(),
                x: 8.0,
                y: 0.5,
                z: 8.0,
                sx: 1.0,
                sy: 1.0,
                sz: 1.0,
                physical: true,
                triangles: 1200,
                memory_bytes: 48_000,
            },
        ],
        morale: vec![500, 500],
        balance: BalanceConfig {
            matches: 20,
            match_ticks: 600,
            base_seed: 2040,
            min_win_permille: 350,
            max_win_permille: 650,
        },
        budgets: BudgetCaps {
            max_draws: 64,
            max_triangles: 1_000_000,
            max_memory_bytes: 256 * 1024 * 1024,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tournament_gate_is_registered_for_proof() {
        // Playtest-side registration: the proof harness resolves this gate
        // by name and calls it through the crate root. Removing the
        // re-export fails the type assertion below; renaming the gate fails
        // the name assertion — either way an unwired gate fails here.
        assert_eq!(crate::GATE_NAME, tournament::GATE_NAME);
        assert_eq!(crate::GATE_NAME, "pq2-balance-tournament");
        let entry: fn() -> Result<tournament::GateOutcome, Failure> = crate::proof_gate;
        let _ = entry;
    }

    #[test]
    fn exploit_gate_is_registered_for_proof() {
        // Same registration shape as the tournament gate: the proof harness
        // resolves the sweep by name and calls it through the crate root.
        assert_eq!(crate::EXPLOIT_GATE_NAME, exploit::EXPLOIT_GATE_NAME);
        assert_eq!(crate::EXPLOIT_GATE_NAME, "pq10-exploit-sweep");
        let entry: fn() -> Result<exploit::ExploitGateOutcome, exploit::SweepError> =
            crate::exploit_gate;
        let _ = entry;
    }

    #[test]
    fn reference_passes_verify() {
        let verdict = verify(&reference_arena());
        assert!(verdict.pass, "reference must pass: {:?}", verdict.failures);
        assert!(verdict.failures.is_empty());
    }

    #[test]
    fn same_variant_twice_gives_same_verdict() {
        let variant = ArenaVariant {
            balance: BalanceConfig {
                matches: 4,
                match_ticks: 60,
                ..reference_arena().balance
            },
            ..reference_arena()
        };
        let first = verify(&variant);
        let second = verify(&variant);
        assert_eq!(first, second);
        assert!(first.pass, "small variant must pass: {:?}", first.failures);
    }

    #[test]
    fn failures_carry_verifier_names_in_fixed_order() {
        // Sealed gate (reachability) plus blown budgets: both verifiers fail
        // and reachability sorts first.
        let mut variant = reference_arena();
        variant.balance.matches = 2;
        variant.balance.match_ticks = 60;
        variant.balance.min_win_permille = 0;
        variant.balance.max_win_permille = 1000;
        // Wall across the middle at x = 0 seals both gates from both spawns.
        variant.props.push(Prop {
            id: 99,
            name: "wall".to_string(),
            x: 0.0,
            y: 1.0,
            z: 0.0,
            sx: 1.0,
            sy: 2.0,
            sz: 40.0,
            physical: true,
            triangles: 2,
            memory_bytes: 8,
        });
        variant.budgets.max_draws = 0;
        let verdict = verify(&variant);
        assert!(!verdict.pass);
        let names: Vec<&str> = verdict.failures.iter().map(|f| f.verifier).collect();
        assert!(
            names.contains(&reachability::VERIFIER),
            "reachability must fail: {names:?}"
        );
        assert!(
            names.contains(&budgets::VERIFIER),
            "budgets must fail: {names:?}"
        );
        let first_reach = names.iter().position(|n| *n == reachability::VERIFIER);
        let first_budget = names.iter().position(|n| *n == budgets::VERIFIER);
        assert!(
            first_reach < first_budget,
            "verifier order must hold: {names:?}"
        );
    }

    /// Reference arena with a small, always-passing tournament so the
    /// verify tests stay fast.
    fn small_reference() -> ArenaVariant {
        let mut variant = reference_arena();
        variant.balance.matches = 2;
        variant.balance.match_ticks = 60;
        variant.balance.min_win_permille = 0;
        variant.balance.max_win_permille = 1000;
        variant
    }

    #[test]
    fn zero_checks_is_never_a_pass() {
        let verdict = Verdict::from_checks(Vec::new(), 0);
        assert!(!verdict.pass);
        assert_eq!(verdict.checks_run, 0);
        assert_eq!(verdict.failures.len(), 1);
        assert_eq!(verdict.failures[0].verifier, NO_CHECKS);
        assert!(verdict.failures[0].reason.contains("no checks ran"));
    }

    #[test]
    fn one_passing_check_is_a_pass() {
        let verdict = Verdict::from_checks(Vec::new(), 1);
        assert!(verdict.pass);
        assert!(verdict.failures.is_empty());
        assert_eq!(verdict.checks_run, 1);
    }

    #[test]
    fn failing_check_still_fails() {
        let failure = Failure {
            verifier: "Test",
            reason: "seeded failure".to_string(),
        };
        let verdict = Verdict::from_checks(vec![failure.clone()], 1);
        assert!(!verdict.pass);
        assert_eq!(verdict.failures, vec![failure]);
        assert_eq!(verdict.checks_run, 1);
    }

    #[test]
    fn verify_counts_three_checks_and_prints_them() {
        let pass = verify(&small_reference());
        assert!(pass.pass, "{:?}", pass.failures);
        assert_eq!(pass.checks_run, 3);
        assert_eq!(pass.to_string(), "PASS checks_run=3 failures=0");
        assert!(format!("{pass:?}").contains("checks_run: 3"));

        let mut variant = small_reference();
        variant.budgets.max_draws = 0;
        let fail = verify(&variant);
        assert!(!fail.pass);
        assert_eq!(fail.checks_run, 3);
        assert_eq!(fail.to_string(), "FAIL checks_run=3 failures=1");
    }
}
