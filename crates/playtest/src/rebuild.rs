//! Checksummed sparse rebuilds for the brief-to-game eval (EV16).
//!
//! A recorded match keeps one checksum per tick plus a full-state snapshot
//! only every [`STRIDE`] ticks (rollback discipline: store little, replay
//! from the nearest snapshot). The recorder is trusted; the record it hands
//! out, a [`Claim`] of tape plus per-tick checksums, is what an audit
//! doubts. [`audit`] replays the claimed tape from sparse snapshots and
//! names the first tick whose checksum differs, in a handful of partial
//! replays instead of one full one.
//!
//! Consumer: `brief_eval` rebuilds every brief's match through [`take`] and
//! [`audit`]. Data owner: the arena owns snapshots and checkpoints; this
//! module owns the stride and the search. Units: fixed ticks. Memory: every
//! snapshot of one match has the same size, so [`Keyframes::memory_permille`]
//! counts snapshots against one per tick. Determinism scope: same-machine
//! reproducibility, like the arena hashes the checksums are. Errors: `None`
//! for an invalid config, tape or record, never a partial answer.

use engine_arena::{resume, run_prefix, ArenaConfig, BodyTape, MatchSnapshot};

/// Ticks between two sparse snapshots.
pub const STRIDE: u32 = 8;

/// The trusted side of a recording: sparse snapshots of the true timeline.
pub struct Keyframes {
    config: ArenaConfig,
    frames: Vec<MatchSnapshot>,
}

/// The record under audit: the tape a match ran and the checksum after each
/// of its ticks.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Claim {
    /// Tape the match ran.
    pub tape: BodyTape,
    /// Checkpoint hash after each tick, `config.ticks` entries.
    pub checksums: Vec<u64>,
}

/// Verdict of one audit.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Audit {
    /// First tick whose rebuilt checksum differs from the claim, `None` for a
    /// clean record.
    pub first_bad: Option<u32>,
    /// Last checksum of the rebuild from the final snapshot.
    pub last: u64,
    /// Partial replays the search spent.
    pub probes: u32,
}

/// Play `tape` once, keep a snapshot every `stride` ticks and the checksum of
/// every tick. `None` for an invalid config or tape or a zero stride.
pub fn take(config: &ArenaConfig, tape: &BodyTape, stride: u32) -> Option<(Keyframes, Claim)> {
    if stride == 0 {
        return None;
    }
    let (_, points) = run_prefix(config, tape, config.ticks)?;
    let checksums: Vec<u64> = points.iter().map(|point| point.hash).collect();
    let frames = (0..config.ticks)
        .step_by(usize::try_from(stride).ok()?)
        .map(|tick| run_prefix(config, tape, tick).map(|(snapshot, _)| snapshot))
        .collect::<Option<Vec<_>>>()?;
    let claim = Claim {
        tape: tape.clone(),
        checksums,
    };
    Some((
        Keyframes {
            config: *config,
            frames,
        },
        claim,
    ))
}

impl Keyframes {
    /// Snapshots held.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// True when no snapshot is held.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Memory against a snapshot per tick, in permille.
    pub fn memory_permille(&self) -> u32 {
        let held = u32::try_from(self.frames.len()).unwrap_or(u32::MAX);
        held.saturating_mul(1000) / self.config.ticks.max(1)
    }
}

/// Audit `claim` against the trusted `keys`. Replaying the claimed tape from
/// a snapshot shows every checksum difference at or after that snapshot and
/// none before it, so "some difference" is monotone over the snapshot
/// index: one probe of the last snapshot, one of the first, then a binary
/// search for the latest snapshot that still sees a difference. That
/// snapshot's rebuild names the exact first bad tick. `None` when the claim
/// does not cover the match or the tape is refused.
pub fn audit(keys: Keyframes, claim: &Claim) -> Option<Audit> {
    let Keyframes { config, frames } = keys;
    let ticks = usize::try_from(config.ticks).ok()?;
    if claim.checksums.len() != ticks || frames.is_empty() {
        return None;
    }
    let top = frames.len() - 1;
    let mut frames: Vec<Option<MatchSnapshot>> = frames.into_iter().map(Some).collect();
    let mut probes = 0;
    let mut last = 0;
    let mut probe = |index: usize| -> Option<Option<u32>> {
        let snapshot = frames.get_mut(index)?.take()?;
        let suffix = resume(snapshot, &config, &claim.tape);
        probes += 1;
        last = suffix.last()?.hash;
        Some(suffix.iter().find_map(|point| {
            let claimed = claim.checksums.get(usize::try_from(point.tick).ok()?)?;
            (*claimed != point.hash).then_some(point.tick)
        }))
    };
    let mut first_bad = probe(top)?;
    if first_bad.is_none() {
        if let Some(bad) = probe(0)? {
            first_bad = Some(bad);
            let (mut low, mut high) = (0, top);
            while high - low > 1 {
                let mid = low + (high - low) / 2;
                match probe(mid)? {
                    Some(bad) => {
                        low = mid;
                        first_bad = Some(bad);
                    }
                    None => high = mid,
                }
            }
        }
    }
    Some(Audit {
        first_bad,
        last,
        probes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_arena::{run, BodyId, BodyInput};

    fn config(seed: u64) -> ArenaConfig {
        ArenaConfig::try_new(seed, 120, 2, 20.0).expect("valid config")
    }

    /// A one-tick walk order the brains did not plan: moves a fighter at once.
    fn nudge(tick: u32) -> BodyInput {
        BodyInput {
            tick,
            fighter: BodyId(0),
            mx: 100,
            mz: 0,
            facing: 0,
            buttons: Default::default(),
        }
    }

    /// First tick where a from-zero run of `tape` differs from `checksums`.
    fn oracle(config: &ArenaConfig, tape: &BodyTape, checksums: &[u64]) -> Option<u32> {
        let rebuilt = run(config, tape);
        assert_eq!(rebuilt.len(), checksums.len());
        rebuilt
            .iter()
            .zip(checksums)
            .find(|(point, claimed)| point.hash != **claimed)
            .map(|(point, _)| point.tick)
    }

    #[test]
    fn a_clean_record_audits_clean_in_two_probes_on_half_the_memory() {
        let config = config(5000);
        let (keys, claim) = take(&config, &BodyTape::default(), STRIDE).expect("recorded");
        assert_eq!(keys.len(), 15);
        assert!(keys.memory_permille() <= 500, "{}", keys.memory_permille());
        let audit = audit(keys, &claim).expect("audited");
        assert_eq!(audit.first_bad, None);
        assert_eq!(audit.probes, 2);
        assert_eq!(Some(&audit.last), claim.checksums.last());
    }

    #[test]
    fn twenty_planted_corruptions_are_located_to_the_tick() {
        for plant in 0..20u32 {
            let config = config(5000 + u64::from(plant));
            let at = 3 + 5 * plant;
            let (keys, mut claim) = take(&config, &BodyTape::default(), STRIDE).expect("recorded");
            let truth = claim.clone();
            let expected = if plant % 2 == 0 {
                // A tape nobody recorded: the nudge moves the fighter at `at`.
                claim.tape = BodyTape(vec![nudge(at)]);
                oracle(&config, &claim.tape, &truth.checksums)
            } else {
                // A checksum that lies at exactly one tick.
                claim.checksums[usize::try_from(at).unwrap()] ^= 1 << plant;
                Some(at)
            };
            assert!(expected.is_some(), "plant {plant} must change something");
            let found = audit(keys, &claim).expect("audited");
            assert_eq!(found.first_bad, expected, "plant {plant} at tick {at}");
            assert!(
                found.probes <= 6,
                "plant {plant} spent {} probes",
                found.probes
            );
        }
    }

    #[test]
    fn the_audit_reproduces_twice_and_refuses_a_short_claim() {
        let config = config(5003);
        let planted = BodyTape(vec![nudge(41)]);
        let audits: Vec<_> = (0..2)
            .map(|_| {
                let (keys, mut claim) =
                    take(&config, &BodyTape::default(), STRIDE).expect("recorded");
                claim.tape = planted.clone();
                audit(keys, &claim).expect("audited")
            })
            .collect();
        assert_eq!(audits[0], audits[1]);
        let (keys, mut claim) = take(&config, &BodyTape::default(), STRIDE).expect("recorded");
        claim.checksums.pop();
        assert_eq!(audit(keys, &claim), None);
        assert!(take(&config, &BodyTape::default(), 0).is_none());
    }
}
