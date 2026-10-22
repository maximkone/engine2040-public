//! Anti-vacuity control for digest gates: refuse a digest that never read
//! the simulation.
//!
//! Owner: engine-proof; consumer: [`crate::a9::render_report`] runs
//! [`control`] beside the golden line, and digest equalities feed
//! [`check`] first. Determinism: integer compares plus fixed
//! [`crate::record`] probes, so the same binary gives the same verdict.
//! Threat model: stubs, not adversaries. A digest replaced by a constant, a
//! "run" that stepped nothing, or a digest that folds the input bytes
//! without simulating must fail here instead of passing a golden equality.
//!
//! The method is probe-and-perturb: step a baseline, flip the smallest unit
//! the input type allows (one ULP of an `f32` velocity), step one tick more,
//! and require the digest to move at each probe and to sit off the two
//! constants a stub leaves (zero, the FNV-1a64 offset basis).

use crate::{record, Config, Input, Tape};
use engine_core::hash::FNV64_OFFSET;
use std::fmt;

/// A digest plus the recorded steps behind it, as an equality gate must
/// state both before the compare counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DigestFacts {
    /// The digest under control (one checkpoint scope).
    pub digest: u64,
    /// Fixed steps whose checkpoints produced the digest. Zero proves nothing.
    pub steps: u32,
}

impl DigestFacts {
    /// Facts for the tape's last legacy checkpoint. `steps` counts recorded
    /// checkpoints, not the tick count the config claims, so a tape that
    /// recorded nothing reports zero steps.
    #[must_use]
    pub fn last_legacy(tape: &Tape) -> Self {
        Self {
            digest: legacy_scope(tape),
            steps: u32::try_from(tape.checkpoints.len()).unwrap_or(0),
        }
    }
}

/// The legacy scope of a tape: the last checkpoint's trajectory digest, the
/// value [`GOLDEN_600`](crate::GOLDEN_600) compares. A tape without
/// checkpoints reads `0`, which [`check`] refuses.
#[must_use]
pub fn legacy_scope(tape: &Tape) -> u64 {
    tape.checkpoints.last().map_or(0, |c| c.legacy)
}

/// The state scope of a tape: the last checkpoint's full-state digest, the
/// value [`replay`](crate::replay) divergence checks.
#[must_use]
pub fn state_scope(tape: &Tape) -> u64 {
    tape.checkpoints.last().map_or(0, |c| c.state)
}

/// Why a digest cannot have come from stepping the simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vacuity {
    /// Zero steps recorded: nothing but initial state could feed the digest.
    NoSteps,
    /// The digest equals the FNV-1a64 offset basis: the hash of no bytes.
    AtOffsetBasis,
    /// The digest is zero: the value a stubbed or skipped computation leaves.
    AtZero,
    /// The digest did not move under a probe perturbation that must reach
    /// the simulation (`what` names it): it reads something else.
    Unmoved(&'static str),
    /// A fixed control probe refused to record: the fixture itself is not
    /// running, so no digest over it can prove anything.
    ProbeFailed,
}

impl fmt::Display for Vacuity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSteps => write!(f, "no steps recorded behind the digest"),
            Self::AtOffsetBasis => {
                write!(f, "digest is the FNV-1a64 offset basis (hash of nothing)")
            }
            Self::AtZero => write!(f, "digest is zero (constant placeholder)"),
            Self::Unmoved(what) => write!(f, "digest did not move when {what} changed"),
            Self::ProbeFailed => write!(f, "a control probe refused to record"),
        }
    }
}

impl std::error::Error for Vacuity {}

/// Face-value guard: the digest must sit behind recorded steps and off the
/// two constants a stub leaves. Call before every digest equality. A real
/// trajectory that happened to land on a refused value is rejected too;
/// that is the price of a cheap, always-on control.
pub fn check(facts: DigestFacts) -> Result<(), Vacuity> {
    if facts.steps == 0 {
        return Err(Vacuity::NoSteps);
    }
    if facts.digest == FNV64_OFFSET {
        return Err(Vacuity::AtOffsetBasis);
    }
    if facts.digest == 0 {
        return Err(Vacuity::AtZero);
    }
    Ok(())
}

/// Control probe length: small enough for a report line, long enough that
/// every probe steps real ticks. Seed 2040 with a non-default tick count,
/// so probes never enter the frozen golden path.
const CONTROL_TICKS: u32 = 8;

/// The probes' shared tick-0 input: body `slot`, velocity `(2.0, 0.0)` m/s.
/// Every field stays inside the fixture's validation bounds.
fn probe_inputs(slot: u32) -> [Input; 1] {
    [Input {
        tick: 0,
        body: slot,
        vx: 2.0,
        vy: 0.0,
    }]
}

/// The baseline input with one unit in the last place of the velocity's
/// `f32` bit pattern flipped: the smallest change the type allows.
fn nudged_inputs() -> [Input; 1] {
    let mut input = probe_inputs(0)[0];
    input.vx = f32::from_bits(input.vx.to_bits() ^ 1);
    [input]
}

/// Record one fixed control probe. The probes are closed constants
/// (tick 0, body < 100, small finite velocities, `ticks` in 8..=9) and only
/// the golden path could refuse them, so a refusal here is a fixture bug,
/// reported as [`Vacuity::ProbeFailed`] rather than a panic.
fn probe_tape(ticks: u32, inputs: &[Input]) -> Result<Tape, Vacuity> {
    record(Config { seed: 2040, ticks }, inputs).map_err(|_| Vacuity::ProbeFailed)
}

/// Facts for one digest computed from one tape.
fn facts_for(tape: &Tape, digest: u64) -> DigestFacts {
    DigestFacts {
        digest,
        steps: u32::try_from(tape.checkpoints.len()).unwrap_or(0),
    }
}

/// Run the anti-vacuity control on a digest producer over the fixture.
///
/// Three fixed probes through [`record`](crate::record): a baseline (one
/// tick-0 input), the same tape with one velocity bit flipped, and one
/// extra tick at the baseline inputs. Every probe digest must clear
/// [`check`]; the flipped-bit digest must differ from the baseline (a
/// digest that ignores its input fails here) and the longer run must
/// differ too (a digest that only folds the input bytes survives the flip
/// and fails here). Units: fixed ticks, m/s; determinism: same binary,
/// same verdict. Errors: [`Vacuity`] naming the first refusal, never a
/// silent pass.
pub fn control(mut digest: impl FnMut(&Tape) -> u64) -> Result<(), Vacuity> {
    let baseline = probe_tape(CONTROL_TICKS, &probe_inputs(0))?;
    let flipped = probe_tape(CONTROL_TICKS, &nudged_inputs())?;
    let longer = probe_tape(CONTROL_TICKS + 1, &probe_inputs(0))?;
    let first = digest(&baseline);
    check(facts_for(&baseline, first))?;
    let nudged = digest(&flipped);
    if nudged == first {
        return Err(Vacuity::Unmoved("one velocity bit"));
    }
    check(facts_for(&flipped, nudged))?;
    let stepped = digest(&longer);
    if stepped == first {
        return Err(Vacuity::Unmoved("one more tick"));
    }
    check(facts_for(&longer, stepped))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{encode_trace_inputs, replay, GOLDEN_600};
    use engine_core::hash_bytes;

    #[test]
    fn one_velocity_bit_moves_both_digest_scopes() {
        // Smallest-unit sensitivity: a one-ULP flip at tick 0 must reach
        // both scopes at the first checkpoint, not merely by the last.
        let base = probe_tape(CONTROL_TICKS, &probe_inputs(0)).expect("probe records");
        let flipped = probe_tape(CONTROL_TICKS, &nudged_inputs()).expect("probe records");
        assert_eq!(nudged_inputs()[0].vx.to_bits(), 2.0f32.to_bits() ^ 1);
        assert_ne!(
            base.checkpoints[0].legacy, flipped.checkpoints[0].legacy,
            "the legacy digest ignored the tick-0 velocity bit"
        );
        assert_ne!(
            base.checkpoints[0].state, flipped.checkpoints[0].state,
            "the state digest ignored the tick-0 velocity bit"
        );
        assert_ne!(
            legacy_scope(&base),
            legacy_scope(&flipped),
            "the final legacy digest ignored the velocity bit"
        );
        assert_ne!(
            state_scope(&base),
            state_scope(&flipped),
            "the final state digest ignored the velocity bit"
        );
        assert_eq!(replay(&flipped), Ok(()));
        check(DigestFacts::last_legacy(&base)).expect("a stepped tape clears the guard");
        check(DigestFacts::last_legacy(&flipped)).expect("the flipped tape clears the guard");
    }

    #[test]
    fn the_trajectory_digest_reads_only_the_steps_it_covers() {
        // Irrelevance control: a step beyond a checkpoint cannot move the
        // tick-5 legacy digest. The state scope folds the tick count by
        // design, so it does move: the two scopes claim different things.
        let five = probe_tape(5, &[]).expect("probe records");
        let six = probe_tape(6, &[]).expect("probe records");
        assert_eq!(
            five.checkpoints[4].legacy, six.checkpoints[4].legacy,
            "a step after the digest moved a checkpoint it never ran"
        );
        assert_ne!(
            five.checkpoints[4].state, six.checkpoints[4].state,
            "the state scope must cover the run length, as its docs say"
        );
        let again = probe_tape(5, &[]).expect("probe records");
        assert_eq!(
            five.checkpoints, again.checkpoints,
            "the fixture must be deterministic"
        );
    }

    #[test]
    fn guard_refuses_digests_that_never_read_the_sim() {
        assert_eq!(
            check(DigestFacts {
                digest: GOLDEN_600,
                steps: 0,
            }),
            Err(Vacuity::NoSteps),
            "a real digest value behind zero steps proves nothing"
        );
        assert_eq!(
            check(DigestFacts {
                digest: 0,
                steps: 600,
            }),
            Err(Vacuity::AtZero)
        );
        assert_eq!(
            check(DigestFacts {
                digest: FNV64_OFFSET,
                steps: 600,
            }),
            Err(Vacuity::AtOffsetBasis),
            "the hash of no bytes is not a trajectory"
        );
        assert_eq!(hash_bytes(&[]), FNV64_OFFSET);
        let stub = Tape {
            config: Config::default(),
            inputs: Vec::new(),
            checkpoints: Vec::new(),
        };
        assert_eq!(legacy_scope(&stub), 0);
        assert_eq!(
            check(DigestFacts::last_legacy(&stub)),
            Err(Vacuity::NoSteps)
        );
    }

    #[test]
    fn control_refuses_a_constant_digest() {
        // The falsifiable half of the guard: a digest replaced by a constant
        // keeps every golden equality green, so the control drives it
        // through the probes and must refuse it.
        assert_eq!(control(|_| 0), Err(Vacuity::AtZero));
        assert_eq!(
            control(|_| GOLDEN_600),
            Err(Vacuity::Unmoved("one velocity bit"))
        );
        // A digest that reads the tape's metadata instead of its trajectory
        // is constant across probes that differ only in input.
        assert_eq!(
            control(|tape| u64::from(tape.config.ticks)),
            Err(Vacuity::Unmoved("one velocity bit"))
        );
    }

    #[test]
    fn control_refuses_a_digest_that_ignores_the_steps() {
        // Folding only the input bytes survives the bit flip (the bytes
        // differ) but not the extra tick (same bytes, more sim): a digest
        // like this would fake a determinism gate.
        let error = control(|tape| hash_bytes(&encode_trace_inputs(&tape.inputs)));
        assert_eq!(error, Err(Vacuity::Unmoved("one more tick")));
    }

    #[test]
    fn fixture_scopes_clear_the_control() {
        control(legacy_scope).expect("the legacy trajectory scope must read the sim");
        control(state_scope).expect("the full-state scope must read the sim");
    }

    #[test]
    fn golden_tape_clears_the_guard() {
        let tape = record(Config::default(), &[]).expect("the default tape records");
        replay(&tape).expect("the default tape replays");
        let facts = DigestFacts::last_legacy(&tape);
        assert_eq!(
            facts.steps,
            Config::default().ticks,
            "one checkpoint per tick"
        );
        check(facts).expect("the golden digest must sit on 600 real steps");
        assert_eq!(facts.digest, GOLDEN_600);
    }
}
