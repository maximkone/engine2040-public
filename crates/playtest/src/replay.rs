//! Tape replay: verify a resimulation against its recorded hash chain.
//!
//! Consumer: the future `replay.verify` / `replay.bisect` door verbs (the
//! endpoint verify pack); this slice supplies the verb bodies, nothing
//! samples them yet. Data owner: the caller owns the config, tape and chain;
//! this module borrows. Units: fixed ticks, integer hashes. Determinism:
//! same-machine reproducibility — the same inputs twice give the same outcome
//! (deterministic [`engine_arena::run`], integer compares, no wall clock).
//! Errors: [`ReplayError`]; corrupt inputs fail typed, never panic.

use core::fmt;

use engine_arena::{ArenaConfig, Checkpoint};

use crate::agent::BodyTape;

/// First tick where a resimulation leaves its recorded chain: the tick plus
/// the recorded hash and the resimulated hash there. A length mismatch reads
/// as divergence at the shared prefix end (the missing side hashes as `0`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReplayDivergence {
    /// Fixed tick whose hash first differs from the recorded chain.
    pub tick: u32,
    /// Recorded chain hash at the divergent tick (`0` past its end).
    pub expected: u64,
    /// Resimulated hash at the divergent tick (`0` past its end).
    pub observed: u64,
}

/// Tape-replay failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReplayError {
    /// Invalid config, invalid tape, or empty resimulation, never partial.
    CorruptTape,
    /// No recorded chain to compare against (empty chain: hashes are never
    /// invented), or — for [`bisect_tape`] only — a clean resimulation with
    /// no divergent tick to name.
    NoChain,
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CorruptTape => write!(f, "corrupt tape"),
            Self::NoChain => write!(f, "no recorded chain"),
        }
    }
}

impl std::error::Error for ReplayError {}

/// Verify `tape` under `config` against the recorded per-tick `chain`:
/// resimulate with [`engine_arena::run`] and compare each tick hash with the
/// recorded chain. Units: fixed ticks. Deterministic: the same inputs twice
/// give the same outcome (deterministic resimulation, no wall clock, only
/// tape actions enter the run). Returns `Ok(None)` when the resimulation
/// matches the chain; `Ok(Some(_))` carries the first divergent tick with
/// both hashes; [`ReplayError::CorruptTape`] covers invalid configs, invalid
/// tapes and empty runs; [`ReplayError::NoChain`] covers a missing chain.
pub fn verify_tape(
    config: &ArenaConfig,
    tape: &BodyTape,
    chain: &[Checkpoint],
) -> Result<Option<ReplayDivergence>, ReplayError> {
    validate_tape(config, tape, chain)?;
    let resim = engine_arena::run(config, tape);
    if resim.is_empty() {
        return Err(ReplayError::CorruptTape);
    }
    for (index, (left, right)) in resim.iter().zip(chain.iter()).enumerate() {
        if left.hash != right.hash {
            let tick = u32::try_from(index).map_err(|_| ReplayError::CorruptTape)?;
            return Ok(Some(ReplayDivergence {
                tick,
                expected: right.hash,
                observed: left.hash,
            }));
        }
    }
    if resim.len() == chain.len() {
        return Ok(None);
    }
    let shared = resim.len().min(chain.len());
    let tick = u32::try_from(shared).map_err(|_| ReplayError::CorruptTape)?;
    Ok(Some(ReplayDivergence {
        tick,
        expected: hash_at(chain, shared),
        observed: hash_at(resim.as_slice(), shared),
    }))
}

/// Name the first tick whose resimulated hash leaves the recorded chain:
/// resimulate with [`engine_arena::run`], then binary-search over the resim
/// checkpoints for the first divergent hash (long tapes bisect instead of
/// scanning). Units: fixed ticks. Deterministic: the same inputs twice give
/// the same tick (deterministic resimulation, no wall clock, only tape
/// actions enter the run). Returns [`ReplayError::NoChain`] for a missing
/// chain or a clean resimulation; corrupt configs, out-of-range tapes or
/// empty runs give [`ReplayError::CorruptTape`].
pub fn bisect_tape(
    config: &ArenaConfig,
    tape: &BodyTape,
    chain: &[Checkpoint],
) -> Result<ReplayDivergence, ReplayError> {
    validate_tape(config, tape, chain)?;
    let resim = engine_arena::run(config, tape);
    if resim.is_empty() {
        return Err(ReplayError::CorruptTape);
    }
    bisect_divergence(resim.as_slice(), chain).ok_or(ReplayError::NoChain)
}

/// Printable one-line `replay.verify` verdict: `clean` with the tick count
/// when the resimulation matches the recorded chain, otherwise the first
/// divergent tick with both hashes. Units: fixed ticks. Deterministic: the
/// same inputs twice print the same line (no wall clock). Cold path: formats
/// one line off the tick; the single resimulation dominates.
pub fn replay_verify(config: &ArenaConfig, tape: &BodyTape, chain: &[Checkpoint]) -> String {
    match verify_tape(config, tape, chain) {
        Ok(None) => format!("replay.verify clean ticks={}", chain.len()),
        Ok(Some(hit)) => format!(
            "replay.verify divergent tick={} expected={:016x} observed={:016x}",
            hit.tick, hit.expected, hit.observed
        ),
        Err(ReplayError::CorruptTape) => "replay.verify corrupt".to_owned(),
        Err(ReplayError::NoChain) => "replay.verify no_chain".to_owned(),
    }
}

/// Printable one-line `replay.bisect` verdict: the first tick whose hash
/// leaves the recorded chain with both hashes, `clean` when nothing leaves,
/// `no_chain` when no chain was recorded. Units: fixed ticks.
/// Deterministic: the same inputs twice print the same line (no wall clock).
/// Cold path: formats one line off the tick; the single resimulation
/// dominates.
pub fn replay_bisect(config: &ArenaConfig, tape: &BodyTape, chain: &[Checkpoint]) -> String {
    match bisect_tape(config, tape, chain) {
        Ok(hit) => format!(
            "replay.bisect tick={} expected={:016x} observed={:016x}",
            hit.tick, hit.expected, hit.observed
        ),
        Err(ReplayError::NoChain) if chain.is_empty() => "replay.bisect no_chain".to_owned(),
        Err(ReplayError::NoChain) => format!("replay.bisect clean ticks={}", chain.len()),
        Err(ReplayError::CorruptTape) => "replay.bisect corrupt".to_owned(),
    }
}

/// Arena-level validation for one tape plus its recorded chain: per-input
/// ranges plus order (the tape contract) and arena bounds (every tick below
/// `ticks`, every slot below `fighters`), mirroring [`engine_arena::run`],
/// then the chain itself (an empty chain is [`ReplayError::NoChain` — hashes
/// are never invented to fill it).
fn validate_tape(
    config: &ArenaConfig,
    tape: &BodyTape,
    chain: &[Checkpoint],
) -> Result<(), ReplayError> {
    if config.validate().is_err() {
        return Err(ReplayError::CorruptTape);
    }
    if tape.validate().is_err() {
        return Err(ReplayError::CorruptTape);
    }
    if tape
        .0
        .iter()
        .any(|input| input.tick >= config.ticks || input.fighter.0 >= config.fighters)
    {
        return Err(ReplayError::CorruptTape);
    }
    if chain.is_empty() {
        return Err(ReplayError::NoChain);
    }
    Ok(())
}

/// Hash at `index`, or `0` past the end (length mismatch reads as divergence
/// at the shared prefix end, mirroring the causal checkpoint rule).
fn hash_at(points: &[Checkpoint], index: usize) -> u64 {
    points.get(index).map_or(0, |point| point.hash)
}

/// True when the first `len` resimulated hashes equal the recorded chain.
/// Callers hold `len <= min(lens)`, so the slices never overrun.
fn prefix_matches(resim: &[Checkpoint], chain: &[Checkpoint], len: usize) -> bool {
    resim[..len]
        .iter()
        .zip(chain[..len].iter())
        .all(|(left, right)| left.hash == right.hash)
}

/// Binary search for the first divergent index in `[0, shared)`: `shared`
/// mismatches while tick `0` vacuously matches, so the answer always exists.
/// Predicate `prefix len matches` is monotone (a matching prefix implies all
/// shorter prefixes match), so the search stays correct even if later hashes
/// reconverge.
fn search_first(resim: &[Checkpoint], chain: &[Checkpoint], shared: usize) -> usize {
    let mut lo = 0usize;
    let mut hi = shared;
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if prefix_matches(resim, chain, mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// First divergence between a resimulation and its recorded chain via binary
/// search, or `None` when the two agree exactly. Length mismatch reads as
/// divergence at the shared prefix end.
fn bisect_divergence(resim: &[Checkpoint], chain: &[Checkpoint]) -> Option<ReplayDivergence> {
    let shared = resim.len().min(chain.len());
    if !prefix_matches(resim, chain, shared) {
        let index = search_first(resim, chain, shared);
        let tick = u32::try_from(index).ok()?;
        return Some(ReplayDivergence {
            tick,
            expected: hash_at(chain, index),
            observed: hash_at(resim, index),
        });
    }
    if resim.len() == chain.len() {
        return None;
    }
    let tick = u32::try_from(shared).ok()?;
    Some(ReplayDivergence {
        tick,
        expected: hash_at(chain, shared),
        observed: hash_at(resim, shared),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::decode_tape;
    use crate::seeded_bug::{seeded_bugfile, BASE_SEED, MATCH_TICKS, SEEDED_TICK};

    #[test]
    fn replay_bisect_seeded_names_tick_8() {
        let bug = seeded_bugfile().expect("seeded run files desync");
        let Some((config, control, probe)) = decode_tape(&bug.tape_bytes) else {
            panic!("seeded tape decodes");
        };
        assert_eq!(config.seed, BASE_SEED, "spike pins seed 2040");
        assert_eq!(config.ticks, MATCH_TICKS, "spike pins 400 ticks");
        let chain = engine_arena::run(&config, &control);
        assert!(!chain.is_empty(), "control records a chain");
        let first = bisect_tape(&config, &probe, &chain).expect("seeded probe bisects");
        assert_eq!(first.tick, SEEDED_TICK, "bisect names the seeded tick");
        assert_eq!(first.tick, 8, "seeded tick is 8");
        assert_eq!(first.tick, bug.first_bad_tick, "bisect matches the watcher");
        assert_ne!(first.expected, first.observed, "hashes differ at the tick");
        // Deterministic: the same tape twice bisects identically.
        let second = bisect_tape(&config, &probe, &chain).expect("second bisect matches");
        assert_eq!(first, second, "same inputs give the same tick twice");
        // The same tape verifies divergent at 8, never clean.
        let divergence = verify_tape(&config, &probe, &chain)
            .expect("verify runs")
            .expect("seeded probe diverges");
        assert_eq!(divergence, first, "verify and bisect agree");
        // Printable verdict names the tick on one line.
        let line = replay_bisect(&config, &probe, &chain);
        assert!(line.contains("tick=8"), "bisect prints the tick: {line}");
        assert_eq!(line.lines().count(), 1, "one line: {line}");
    }

    #[test]
    fn replay_verify_clean_passes() {
        let bug = seeded_bugfile().expect("seeded run files desync");
        let Some((config, control, _)) = decode_tape(&bug.tape_bytes) else {
            panic!("seeded tape decodes");
        };
        let chain = engine_arena::run(&config, &control);
        assert!(!chain.is_empty(), "control records a chain");
        assert_eq!(
            verify_tape(&config, &control, &chain).expect("verify runs"),
            None,
            "control versus its own chain verifies with no divergence"
        );
        assert_eq!(
            bisect_tape(&config, &control, &chain),
            Err(ReplayError::NoChain),
            "clean tape has no tick to name"
        );
        let line = replay_verify(&config, &control, &chain);
        assert!(line.contains("clean"), "verify prints clean: {line}");
        assert_eq!(line.lines().count(), 1, "one line: {line}");
        // Deterministic: the same tape twice verifies identically.
        assert_eq!(
            verify_tape(&config, &control, &chain),
            verify_tape(&config, &control, &chain),
            "same inputs give the same verdict twice"
        );
        // A missing chain is refused, never invented.
        assert_eq!(
            verify_tape(&config, &control, &[]),
            Err(ReplayError::NoChain),
            "missing chain must not verify"
        );
        assert_eq!(
            bisect_tape(&config, &control, &[]),
            Err(ReplayError::NoChain),
            "missing chain must not bisect"
        );
        // A corrupt config is refused, never partial.
        let bad = ArenaConfig { ticks: 0, ..config };
        assert_eq!(
            verify_tape(&bad, &control, &chain),
            Err(ReplayError::CorruptTape),
            "bad ticks must fail as corrupt"
        );
        assert_eq!(
            bisect_tape(&bad, &control, &chain),
            Err(ReplayError::CorruptTape),
            "bad ticks must fail as corrupt"
        );
    }
}
