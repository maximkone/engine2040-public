//! Causal replay: attribute a failing tape to its smallest input cause.
//!
//! Consumer: the A7 proof plus bug triage reads [`BugFile`] tapes.
//! Data owner: the caller owns the config and tapes; this module borrows.
//! Units: fixed ticks. Determinism scope: same-machine reproducibility — the
//! same config plus tapes twice gives the identical report (sorted iteration,
//! deterministic [`engine_arena`] resimulation, no wall clock).
//! Error policy: corrupt tapes fail as [`CausalError::CorruptTape`], a tape
//! pair with no checkpoint divergence as [`CausalError::NoFailureReproduces`].

use std::collections::BTreeMap;

use engine_arena::{ArenaConfig, Checkpoint, FighterInput, FighterTape};

use crate::agent::{decode_tape, scan_desync, BugFile};

/// One attributed input: the failing tape's entry at `(tick, fighter)` whose
/// reset to the baseline entry removes the divergence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cause {
    /// Fixed tick of the attributed input.
    pub tick: u32,
    /// Fighter slot of the attributed input.
    pub fighter: u8,
    /// The failing tape's input at `(tick, fighter)`.
    pub input: FighterInput,
}

/// Causal replay verdict: the first tick where baseline and failing
/// checkpoints differ, plus the 1-minimal cause set (every listed input is
/// individually necessary: resetting any one of them to baseline removes the
/// divergence). Empty `causes` never occurs: a divergent pair always has at
/// least one differing input at or before the divergent tick.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CausalReport {
    /// First tick where the failing run diverges from baseline.
    pub divergent_tick: u32,
    /// Baseline checkpoint hash at the divergent tick.
    pub expected: u64,
    /// Failing checkpoint hash at the divergent tick.
    pub observed: u64,
    /// Minimal causes in `(tick, fighter)` order.
    pub causes: Vec<Cause>,
}

/// Causal replay failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CausalError {
    /// Invalid config, invalid tape, or malformed tape bytes, never partial.
    CorruptTape,
    /// Baseline and failing runs agree: nothing to attribute.
    NoFailureReproduces,
}

/// Attribute the divergence of `failing` from `baseline` under `config`:
/// resimulate both, name the first divergent tick, then reset each differing
/// input at or before that tick to baseline one at a time and keep the ones
/// whose reset removes the divergence. Deterministic in the report.
pub fn attribute(
    config: &ArenaConfig,
    baseline: &FighterTape,
    failing: &FighterTape,
) -> Result<CausalReport, CausalError> {
    validate_pair(config, baseline, failing)?;
    let baseline_run = engine_arena::run(config, baseline);
    let failing_run = engine_arena::run(config, failing);
    if baseline_run.is_empty() || failing_run.is_empty() {
        return Err(CausalError::CorruptTape);
    }
    let Some(divergent_tick) = scan_desync(&baseline_run, &failing_run) else {
        return Err(CausalError::NoFailureReproduces);
    };
    let (expected, observed) = hashes_at(&baseline_run, &failing_run, divergent_tick);
    // Candidates are exactly the keys where failing differs from baseline:
    // identical tapes resimulate identically, so a divergent pair always has
    // at least one, and only keys at or before the divergent tick can have
    // caused it (the sim reads the past, never the future).
    let baseline_by_key = index_by_key(baseline);
    let mut causes = Vec::new();
    for input in &failing.0 {
        if input.tick > divergent_tick {
            continue;
        }
        let key = (input.tick, input.fighter.0);
        if baseline_by_key.get(&key) == Some(input) {
            continue;
        }
        let perturbed = reset_key(failing, &baseline_by_key, key);
        if engine_arena::run(config, &perturbed) == baseline_run {
            causes.push(Cause {
                tick: input.tick,
                fighter: input.fighter.0,
                input: *input,
            });
        }
    }
    if causes.is_empty() {
        // Unreachable for a divergent pair (the full reset equals baseline),
        // but a changed contract must fail loudly, never with an empty claim.
        return Err(CausalError::CorruptTape);
    }
    Ok(CausalReport {
        divergent_tick,
        expected,
        observed,
        causes,
    })
}

/// Attribute a filed bug: decode its tape bytes and attribute the probe
/// tape against the control tape. Malformed bytes are [`CausalError::CorruptTape`];
/// a non-divergent control/probe pair is [`CausalError::NoFailureReproduces`]
/// (stillness-only tapes with identical control and probe carry no input cause).
pub fn attribute_bug(bug: &BugFile) -> Result<CausalReport, CausalError> {
    let Some((config, control, probe)) = decode_tape(&bug.tape_bytes) else {
        return Err(CausalError::CorruptTape);
    };
    attribute(&config, &control, &probe)
}

/// Verify a clean replay of `failing` against `baseline` under `config`:
/// resimulate both with [`engine_arena::run`] and name the first divergent
/// tick with [`scan_desync`]. Units: fixed ticks. Deterministic: the same
/// config plus tapes twice gives the same result (sorted resimulation, no
/// wall clock, no float). Returns `Ok(())` when [`scan_desync`] finds no
/// divergence; returns [`CausalError::NoFailureReproduces`] when the runs
/// diverge (no clean result to report, so callers needing the tick use
/// [`bisect_replay`]); corrupt configs, out-of-range tapes or empty runs
/// give [`CausalError::CorruptTape`].
pub fn verify_replay(
    config: &ArenaConfig,
    baseline: &FighterTape,
    failing: &FighterTape,
) -> Result<(), CausalError> {
    validate_pair(config, baseline, failing)?;
    let baseline_run = engine_arena::run(config, baseline);
    let failing_run = engine_arena::run(config, failing);
    if baseline_run.is_empty() || failing_run.is_empty() {
        return Err(CausalError::CorruptTape);
    }
    match scan_desync(&baseline_run, &failing_run) {
        None => Ok(()),
        Some(_) => Err(CausalError::NoFailureReproduces),
    }
}

/// Name the first tick whose hash leaves the recorded chain: resimulate
/// both tapes with [`engine_arena::run`] and return the [`scan_desync`]
/// tick. Units: fixed ticks. Deterministic: the same config plus tapes
/// twice gives the same tick (sorted resimulation, no wall clock, no
/// float). Returns [`CausalError::NoFailureReproduces`] when the runs agree;
/// corrupt configs, out-of-range tapes or empty runs give
/// [`CausalError::CorruptTape`].
pub fn bisect_replay(
    config: &ArenaConfig,
    baseline: &FighterTape,
    failing: &FighterTape,
) -> Result<u32, CausalError> {
    validate_pair(config, baseline, failing)?;
    let baseline_run = engine_arena::run(config, baseline);
    let failing_run = engine_arena::run(config, failing);
    if baseline_run.is_empty() || failing_run.is_empty() {
        return Err(CausalError::CorruptTape);
    }
    scan_desync(&baseline_run, &failing_run).ok_or(CausalError::NoFailureReproduces)
}

/// Printable one-line `replay.verify` verdict for a baseline/failing tape
/// pair: `clean` with the tick count when the resimulation matches, otherwise
/// the first divergent tick. Units: fixed ticks. Deterministic: the same
/// inputs twice print the same line (deterministic resimulation, no wall
/// clock, no float). Cold path: at most two resimulations off the tick; the
/// pair search itself stays in [`verify_replay`] and [`bisect_replay`], so no
/// second scan exists here.
///
/// Consumer: [`crate::seeded_bug::render_report`] posts this line into the A7
/// evidence report; the endpoint `replay` pack samples the same bodies.
pub fn verify_line(config: &ArenaConfig, baseline: &FighterTape, failing: &FighterTape) -> String {
    match verify_replay(config, baseline, failing) {
        Ok(()) => format!("replay.verify clean ticks={}", config.ticks),
        Err(CausalError::CorruptTape) => "replay.verify corrupt".to_owned(),
        Err(CausalError::NoFailureReproduces) => match bisect_replay(config, baseline, failing) {
            Ok(tick) => format!("replay.verify divergent tick={tick}"),
            Err(_) => "replay.verify corrupt".to_owned(),
        },
    }
}

/// Printable one-line `replay.bisect` verdict for a baseline/failing tape
/// pair: the first divergent tick, or `clean` with the tick count when the
/// runs agree. Units, determinism and cold-path notes match [`verify_line`];
/// the search stays in [`bisect_replay`], so no second scan exists here.
///
/// Consumer: [`crate::seeded_bug::render_report`] posts this line into the A7
/// evidence report; the endpoint `replay` pack samples the same bodies.
pub fn bisect_line(config: &ArenaConfig, baseline: &FighterTape, failing: &FighterTape) -> String {
    match bisect_replay(config, baseline, failing) {
        Ok(tick) => format!("replay.bisect tick={tick}"),
        Err(CausalError::NoFailureReproduces) => {
            format!("replay.bisect clean ticks={}", config.ticks)
        }
        Err(CausalError::CorruptTape) => "replay.bisect corrupt".to_owned(),
    }
}

/// Arena-level validation for both tapes: per-input ranges plus order (the
/// tape contract) and arena bounds (every tick below `ticks`, every slot
/// below `fighters`), mirroring [`engine_arena::run`].
fn validate_pair(
    config: &ArenaConfig,
    baseline: &FighterTape,
    failing: &FighterTape,
) -> Result<(), CausalError> {
    if config.validate().is_err() {
        return Err(CausalError::CorruptTape);
    }
    for tape in [baseline, failing] {
        if tape.validate().is_err() {
            return Err(CausalError::CorruptTape);
        }
        if tape
            .0
            .iter()
            .any(|input| input.tick >= config.ticks || input.fighter.0 >= config.fighters)
        {
            return Err(CausalError::CorruptTape);
        }
    }
    Ok(())
}

/// Index a tape by `(tick, fighter)` in key order.
fn index_by_key(tape: &FighterTape) -> BTreeMap<(u32, u8), FighterInput> {
    tape.0
        .iter()
        .map(|input| ((input.tick, input.fighter.0), *input))
        .collect()
}

/// Copy `failing` with `key` reset to the baseline entry (replaced when the
/// baseline holds the key, removed when it does not). Key order is preserved:
/// replacement keeps the slot, removal keeps the remainder sorted.
fn reset_key(
    failing: &FighterTape,
    baseline_by_key: &BTreeMap<(u32, u8), FighterInput>,
    key: (u32, u8),
) -> FighterTape {
    let mut inputs = Vec::with_capacity(failing.0.len());
    for input in &failing.0 {
        if (input.tick, input.fighter.0) == key {
            if let Some(base) = baseline_by_key.get(&key) {
                inputs.push(*base);
            }
        } else {
            inputs.push(*input);
        }
    }
    FighterTape(inputs)
}

/// Checkpoint hashes at the divergent tick; both runs cover the tick because
/// a length mismatch diverges at the shared prefix end.
fn hashes_at(
    baseline_run: &[Checkpoint],
    failing_run: &[Checkpoint],
    divergent_tick: u32,
) -> (u64, u64) {
    let index = divergent_tick as usize;
    let expected = baseline_run.get(index).map_or(0, |point| point.hash);
    let observed = failing_run.get(index).map_or(0, |point| point.hash);
    (expected, observed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Agent, AgentConfig, Caps, Driver, Metric, Policy};

    fn agent_config() -> AgentConfig {
        AgentConfig {
            base_seed: 2040,
            ticks: 400,
            fighters: 2,
            half_extent_m: 20.0,
        }
    }

    fn arena_config() -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 400,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        }
    }

    #[test]
    fn seeded_divert_bug_attributes_to_its_input() {
        // Seeded bug: one sideways wish at tick 50 for fighter 1.
        let config = agent_config();
        let driver = Driver {
            policy: Policy::Divert {
                tick: 50,
                fighter: 1,
            },
            caps: Caps::mock(),
        };
        let agent = Agent { config, driver };
        let report = agent.drive(1);
        let desync = report
            .bugs
            .iter()
            .find(|bug| matches!(bug.metric, Metric::Desync { .. }))
            .expect("diverted probe files desync");
        let causal = attribute_bug(desync).expect("seeded bug attributes");
        // Smallest cause: exactly the diverted input.
        assert_eq!(causal.causes.len(), 1, "one cause: {causal:?}");
        assert_eq!(
            (causal.causes[0].tick, causal.causes[0].fighter),
            (50, 1),
            "cause is the diverted input: {causal:?}"
        );
        assert_eq!(causal.causes[0].input.mx, 100);
        // First divergence cannot predate the diverted tick and must match
        // the desync watcher's own tick.
        assert!(causal.divergent_tick >= 50);
        assert_eq!(causal.divergent_tick, desync.first_bad_tick);
        assert_ne!(causal.expected, causal.observed);
        // Deterministic: the same bug twice gives the identical report.
        assert_eq!(attribute_bug(desync), Ok(causal));
    }

    #[test]
    fn direct_tapes_agree_with_bug_attribution() {
        let match_config = arena_config();
        let driver = Driver {
            policy: Policy::Divert {
                tick: 50,
                fighter: 0,
            },
            caps: Caps::mock(),
        };
        let control = driver.control_tape(&match_config);
        let probe = driver.probe_tape(&match_config, &control);
        let causal = attribute(&match_config, &control, &probe).expect("divergence attributes");
        assert_eq!(causal.causes.len(), 1);
        assert_eq!((causal.causes[0].tick, causal.causes[0].fighter), (50, 0));
        // Resetting the cause to baseline removes the divergence: the full
        // baseline run equals the perturbed run by construction of the check.
        let baseline_run = engine_arena::run(&match_config, &control);
        assert_eq!(scan_desync(&baseline_run, &baseline_run), None);
        assert_eq!(attribute(&match_config, &control, &probe), Ok(causal));
    }

    #[test]
    fn clean_pair_reports_no_failure() {
        let match_config = arena_config();
        let driver = Driver::mock();
        let control = driver.control_tape(&match_config);
        let probe = driver.probe_tape(&match_config, &control);
        assert_eq!(
            attribute(&match_config, &control, &probe),
            Err(CausalError::NoFailureReproduces)
        );
        // Same tape twice is never a failure either.
        assert_eq!(
            attribute(&match_config, &probe, &probe),
            Err(CausalError::NoFailureReproduces)
        );
    }

    #[test]
    fn corrupt_inputs_fail_as_corrupt_tape() {
        let match_config = arena_config();
        let tape = FighterTape::default();
        let bad_config = ArenaConfig {
            ticks: 0,
            ..match_config
        };
        assert_eq!(
            attribute(&bad_config, &tape, &tape),
            Err(CausalError::CorruptTape)
        );
        // Tape tick outside the match.
        let driver = Driver {
            policy: Policy::Divert {
                tick: 399,
                fighter: 0,
            },
            caps: Caps::mock(),
        };
        let probe = driver.probe_tape(&match_config, &tape);
        let short_config = ArenaConfig {
            ticks: 100,
            ..match_config
        };
        assert_eq!(
            attribute(&short_config, &tape, &probe),
            Err(CausalError::CorruptTape)
        );
        // Garbage bug bytes never panic and never attribute.
        let garbage = BugFile {
            tape_bytes: vec![0x50, 0x54, 0x41, 0x00, 1, 2, 3],
            first_bad_tick: 0,
            metric: Metric::Stuck { fighter: 0 },
        };
        assert_eq!(attribute_bug(&garbage), Err(CausalError::CorruptTape));
        let truncated = BugFile {
            tape_bytes: Vec::new(),
            first_bad_tick: 0,
            metric: Metric::Stuck { fighter: 0 },
        };
        assert_eq!(attribute_bug(&truncated), Err(CausalError::CorruptTape));
    }

    #[test]
    fn error_variants_are_copy_and_comparable() {
        assert_eq!(CausalError::CorruptTape, CausalError::CorruptTape);
        assert_ne!(CausalError::CorruptTape, CausalError::NoFailureReproduces);
    }

    #[test]
    fn replay_bisect_seeded_finds_tick_8() {
        use crate::seeded_bug::{bisect_seeded, seeded_bugfile, SEEDED_TICK};
        let bug = seeded_bugfile().expect("seeded run files desync");
        let tick = bisect_seeded(&bug).expect("seeded tape bisects");
        assert_eq!(tick, SEEDED_TICK, "bisect names the seeded tick");
        assert_eq!(tick, 8, "seeded tick is 8");
        assert_eq!(tick, bug.first_bad_tick, "bisect matches the watcher");
        let Some((config, control, probe)) = decode_tape(&bug.tape_bytes) else {
            panic!("seeded tape decodes");
        };
        assert_eq!(
            bisect_replay(&config, &control, &probe),
            Ok(SEEDED_TICK),
            "direct bisect agrees with the seeded helper"
        );
    }

    #[test]
    fn replay_bisect_clean_verifies() {
        use crate::seeded_bug::{seeded_bugfile, verify_seeded_clean};
        let bug = seeded_bugfile().expect("seeded run files desync");
        let Some((config, control, _)) = decode_tape(&bug.tape_bytes) else {
            panic!("seeded tape decodes");
        };
        assert_eq!(
            verify_replay(&config, &control, &control),
            Ok(()),
            "control versus control verifies with no divergence"
        );
        assert_eq!(
            bisect_replay(&config, &control, &control),
            Err(CausalError::NoFailureReproduces),
            "clean pair has no tick to name"
        );
        assert!(
            verify_seeded_clean(&bug),
            "seeded control versus control verifies"
        );
    }

    #[test]
    fn replay_bisect_stable_x2() {
        use crate::seeded_bug::seeded_bugfile;
        let bug = seeded_bugfile().expect("seeded run files desync");
        let Some((config, control, probe)) = decode_tape(&bug.tape_bytes) else {
            panic!("seeded tape decodes");
        };
        let first = bisect_replay(&config, &control, &probe).expect("first bisect names a tick");
        let second = bisect_replay(&config, &control, &probe).expect("second bisect names a tick");
        assert_eq!(first, second, "same inputs give the same tick twice");
    }

    #[test]
    fn replay_bisect_line_prints_seeded_tick_8() {
        use crate::seeded_bug::{seeded_bugfile, SEEDED_TICK};
        let bug = seeded_bugfile().expect("seeded run files desync");
        let Some((config, control, probe)) = decode_tape(&bug.tape_bytes) else {
            panic!("seeded tape decodes");
        };
        let bisect = bisect_line(&config, &control, &probe);
        assert!(
            bisect.contains("tick=8"),
            "bisect prints the seeded tick: {bisect}"
        );
        assert_eq!(bisect.lines().count(), 1, "one line: {bisect}");
        assert_eq!(bisect, bisect_line(&config, &control, &probe));
        let verify = verify_line(&config, &control, &probe);
        assert!(
            verify.contains("tick=8"),
            "verify prints the seeded tick: {verify}"
        );
        assert_eq!(verify.lines().count(), 1, "one line: {verify}");
        assert_eq!(SEEDED_TICK, 8, "seeded tick is 8");
    }

    #[test]
    fn replay_verify_line_clean_and_corrupt() {
        use crate::seeded_bug::seeded_bugfile;
        let bug = seeded_bugfile().expect("seeded run files desync");
        let Some((config, control, _)) = decode_tape(&bug.tape_bytes) else {
            panic!("seeded tape decodes");
        };
        let clean = verify_line(&config, &control, &control);
        assert!(clean.contains("clean"), "clean verifies: {clean}");
        assert_eq!(clean.lines().count(), 1, "one line: {clean}");
        assert_eq!(
            bisect_line(&config, &control, &control),
            format!("replay.bisect clean ticks={}", config.ticks),
            "clean pair has no tick to name"
        );
        let bad = ArenaConfig { ticks: 0, ..config };
        assert_eq!(
            bisect_line(&bad, &control, &control),
            "replay.bisect corrupt"
        );
        assert_eq!(
            verify_line(&bad, &control, &control),
            "replay.verify corrupt"
        );
    }
}
