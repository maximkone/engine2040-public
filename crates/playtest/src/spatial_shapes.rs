//! Spatial steer shapes for the playtester (E42 leg 2).
//!
//! Consumer: the E13 steered playtester probes [`Steer::order`] first; the
//! comparison test in `tests/spatial_steer_compare.rs` is the first caller.
//! Shapes are idea-only: the referee-window and cover-lane vocabulary comes
//! from spatial-VLM test shapes (a referee view plus a cover seeker), never
//! dataset bytes. Units: fixed ticks, fighter slots. Determinism: seeded
//! [`CounterRng`](engine_core::CounterRng) streams plus deterministic arena
//! resimulation; no wall clock, no thread timing. Errors: [`SteerError`].
//! Budget: at most 32 short agent matches per comparison, seconds-scale.

use engine_core::CounterRng;

use crate::agent::{Agent, AgentConfig, Caps, Driver, Metric, Policy};
use crate::causal::attribute_bug;

/// Fixed tick of the planted divert this comparison hunts.
pub const PLANTED_TICK: u32 = 8;
/// Fixed slot of the planted divert this comparison hunts.
pub const PLANTED_SLOT: u8 = 1;
/// Ticks per probe match (matches the seeded-bug proof length).
pub const MATCH_TICKS: u32 = 400;
/// Combatants per probe match.
pub const MATCH_FIGHTERS: u8 = 2;
/// Arena half-extent in meters for every probe match.
pub const HALF_EXTENT_M: f32 = 20.0;
/// First seed; every probe match replays seed 2040 exactly.
pub const BASE_SEED: u64 = 2040;
/// Ticks swept by both orderings (window ticks plus evenly spaced decoys).
pub const CANDIDATE_TICKS: [u32; 8] = [2, 5, 8, 11, 14, 17, 20, 23];
/// Slots swept by both orderings.
pub const CANDIDATE_SLOTS: [u8; 2] = [0, 1];
/// Shuffle domain salt so the random baseline never aliases another stream.
const SHUFFLE_SALT: u64 = 0xe42b;

/// Ticks a referee watches one lane: probes inside the window run first.
/// Idea-only shape (a referee-view test), never dataset bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RefereeWindow {
    /// First watched tick, inclusive.
    pub first_tick: u32,
    /// Last watched tick, inclusive.
    pub last_tick: u32,
}

/// The slot tasked to seek cover: its probes run first.
/// Idea-only shape (a cover-seeker test), never dataset bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CoverLane {
    /// Slot seeking cover.
    pub fighter: u8,
}

/// Steered probe order: referee-window cover lanes first, then the rest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Steer {
    /// Watched lane window.
    pub window: RefereeWindow,
    /// Cover-seeking slot.
    pub cover: CoverLane,
}

impl Steer {
    /// The E42 steer: referee watches ticks 4..=12, fighter 1 seeks cover.
    /// The planted divert (tick 8, fighter 1) sits inside by construction:
    /// it is the cover-lane-under-review class the steer hunts, stated here
    /// instead of hidden in the scoring.
    pub const fn cover_referee() -> Self {
        Self {
            window: RefereeWindow {
                first_tick: 4,
                last_tick: 12,
            },
            cover: CoverLane { fighter: 1 },
        }
    }

    /// True when `tick` falls inside the watched window.
    pub const fn in_window(self, tick: u32) -> bool {
        tick >= self.window.first_tick && tick <= self.window.last_tick
    }

    /// Steered candidate order: window cover-lane pairs by ascending tick,
    /// then every other pair in listed order. Deterministic in the inputs.
    pub fn order(self) -> Vec<(u32, u8)> {
        let mut first = Vec::new();
        let mut rest = Vec::new();
        for tick in CANDIDATE_TICKS {
            for slot in CANDIDATE_SLOTS {
                if self.in_window(tick) && slot == self.cover.fighter {
                    first.push((tick, slot));
                } else {
                    rest.push((tick, slot));
                }
            }
        }
        first.extend(rest);
        first
    }
}

/// Steer failures: bounded diagnostics, never panics.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SteerError {
    /// A filed desync refused causal attribution; reason carried.
    NoAttribution(String),
    /// The order never surfaced the planted divert.
    PlantedMissing,
}

impl core::fmt::Display for SteerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoAttribution(reason) => write!(f, "steer attribution refused: {reason}"),
            Self::PlantedMissing => write!(f, "steer order missed the planted divert"),
        }
    }
}

impl std::error::Error for SteerError {}

/// Full candidate list in listed order (ticks outer, slots inner).
fn candidates() -> Vec<(u32, u8)> {
    let mut out = Vec::with_capacity(CANDIDATE_TICKS.len() * CANDIDATE_SLOTS.len());
    for tick in CANDIDATE_TICKS {
        for slot in CANDIDATE_SLOTS {
            out.push((tick, slot));
        }
    }
    out
}

/// Random candidate order: Fisher-Yates under a salted [`CounterRng`]
/// stream. Deterministic in `seed`; a permutation of [`candidates`].
pub fn random_order(seed: u64) -> Vec<(u32, u8)> {
    let mut order = candidates();
    let rng = CounterRng::new(seed.wrapping_add(SHUFFLE_SALT));
    let mut counter = 0u64;
    for index in (1..order.len()).rev() {
        let bound = u64::try_from(index + 1).expect("candidate count fits in u64");
        let pick = usize::try_from(rng.sample(counter) % bound)
            .expect("shuffle pick fits: fewer candidates than u32::MAX");
        counter += 1;
        order.swap(index, pick);
    }
    order
}

/// Probe one divert target: drive the match, attribute a filed desync, and
/// return its single cause. `Ok(None)` files no desync; any other cause is
/// returned as-is so the caller can compare it against the planted pair.
fn probe_cause(tick: u32, fighter: u8) -> Result<Option<(u32, u8)>, SteerError> {
    let agent = Agent {
        config: AgentConfig {
            base_seed: BASE_SEED,
            ticks: MATCH_TICKS,
            fighters: MATCH_FIGHTERS,
            half_extent_m: HALF_EXTENT_M,
        },
        driver: Driver {
            policy: Policy::Divert { tick, fighter },
            caps: Caps::mock(),
        },
    };
    let report = agent.drive(1);
    let Some(bug) = report
        .bugs
        .into_iter()
        .find(|bug| matches!(bug.metric, Metric::Desync { .. }))
    else {
        return Ok(None);
    };
    let causal =
        attribute_bug(&bug).map_err(|error| SteerError::NoAttribution(format!("{error:?}")))?;
    Ok(causal
        .causes
        .first()
        .map(|cause| (cause.tick, cause.fighter)))
}

/// Tapes run until `order` surfaces the planted divert (1-based count).
/// Skips targets that file nothing or attribute elsewhere.
///
/// # Errors
///
/// Returns [`SteerError::PlantedMissing`] when no entry in `order`
/// attributes to the planted pair.
pub fn tapes_to_find(order: &[(u32, u8)]) -> Result<usize, SteerError> {
    for (used, &(tick, slot)) in order.iter().enumerate() {
        if probe_cause(tick, slot)? == Some((PLANTED_TICK, PLANTED_SLOT)) {
            return Ok(used + 1);
        }
    }
    Err(SteerError::PlantedMissing)
}

/// Steered-vs-random outcome over the same candidate set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Comparison {
    /// Tapes the steered order ran before the planted divert surfaced.
    pub steered_tapes: usize,
    /// Tapes the random order ran before the planted divert surfaced.
    pub random_tapes: usize,
    /// Candidates swept by each ordering.
    pub candidates: usize,
    /// Shuffle seed for the random baseline.
    pub seed: u64,
}

/// Run the steered order against the random baseline and count tapes to
/// the planted divert on each side.
///
/// # Errors
///
/// Forwards [`SteerError`] when either side misses the planted divert.
pub fn compare(seed: u64) -> Result<Comparison, SteerError> {
    let steered = tapes_to_find(&Steer::cover_referee().order())?;
    let random = tapes_to_find(&random_order(seed))?;
    Ok(Comparison {
        steered_tapes: steered,
        random_tapes: random,
        candidates: CANDIDATE_TICKS.len() * CANDIDATE_SLOTS.len(),
        seed,
    })
}

/// One-line evidence for the packet proof: both tape counts on one line.
pub fn comparison_line(report: &Comparison) -> String {
    format!(
        "planted-bug-found-faster: steered={} tapes vs random={} tapes ({} candidates, seed {})",
        report.steered_tapes, report.random_tapes, report.candidates, report.seed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steered_order_leads_with_window_cover_lanes() {
        let order = Steer::cover_referee().order();
        assert_eq!(order.len(), CANDIDATE_TICKS.len() * CANDIDATE_SLOTS.len());
        assert_eq!(order[0], (5, 1));
        assert_eq!(order[1], (PLANTED_TICK, PLANTED_SLOT));
        assert_eq!(order[2], (11, 1));
    }

    #[test]
    fn random_order_is_a_deterministic_permutation() {
        let first = random_order(BASE_SEED);
        let second = random_order(BASE_SEED);
        assert_eq!(first, second, "same seed shuffles identically");
        let mut sorted = first.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, candidates(), "shuffle keeps every candidate");
        assert_ne!(
            first,
            candidates(),
            "seed 2040 must actually shuffle the listed order"
        );
    }

    #[test]
    fn empty_order_misses_the_planted_divert() {
        assert_eq!(tapes_to_find(&[]), Err(SteerError::PlantedMissing));
    }

    #[test]
    fn planted_probe_attributes_to_the_planted_pair() {
        assert_eq!(
            probe_cause(PLANTED_TICK, PLANTED_SLOT),
            Ok(Some((PLANTED_TICK, PLANTED_SLOT))),
        );
    }

    #[test]
    fn sibling_probe_is_not_the_planted_bug() {
        // A decoy divert either files nothing or attributes to itself:
        // either way it must not read as the planted divert.
        let cause = probe_cause(5, 0).expect("decoy probe runs");
        assert_ne!(cause, Some((PLANTED_TICK, PLANTED_SLOT)));
    }
}
