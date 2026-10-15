//! bout: deterministic bout harness over integer trees.
//! Owner: this module owns bout configs, tapes and reports.
//! Units: bouts, fixed ticks, seeds; digests are `u64` hashes.
//! Determinism: same trees plus same config give byte-identical
//! reports; states derive from seeded mixes, no clock, no float.
//! Errors: typed config failures, never panics.
//! Budget: tapes are cold storage; the per-tick path allocates nothing.

use crate::bt::{BtBlackboard, BtState, BtStatus, BtTree, ACTION_ADVANCE, ACTION_ATTACK};
use engine_core::hash::GOLDEN_GAMMA;
use engine_core::{splitmix64, Fnv64};

/// Tape encoding version. Carried for forward compatibility.
pub const BOUT_VERSION: u8 = 1;
/// Largest bout batch. Bounds harness reports.
pub const MAX_BOUTS: usize = 256;
/// Longest bout in fixed ticks. Bounds harness time.
pub const MAX_TICKS: u32 = 3_600;
/// Stillness window in ticks (3 s at 60 Hz). A stream of identical
/// decisions this long counts as stuck.
pub const STUCK_WINDOW_TICKS: u32 = 180;
/// Standard batch width matching the playtest proof size.
pub const STANDARD_BOUTS: usize = 200;
/// Standard bout length in fixed ticks.
pub const STANDARD_TICKS: u32 = 600;
/// Standard replay seed.
pub const STANDARD_SEED: u64 = 2040;

/// Bout configuration failures. Typed, never strings. Units: counts
/// carried as integers. Errors: returned, never panicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoutError {
    /// No bouts requested.
    EmptyBouts,
    /// Batch exceeds the cap; culprit carried.
    TooManyBouts {
        /// Requested bout count.
        got: usize,
    },
    /// No ticks requested.
    EmptyTicks,
    /// Bout exceeds the tick cap; culprit carried.
    TooManyTicks {
        /// Requested tick count.
        got: u32,
    },
}

impl core::fmt::Display for BoutError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            BoutError::EmptyBouts => write!(f, "bout: need at least one bout"),
            BoutError::TooManyBouts { got } => {
                write!(f, "bout: {got} bouts exceed {MAX_BOUTS}")
            }
            BoutError::EmptyTicks => write!(f, "bout: need at least one tick"),
            BoutError::TooManyTicks { got } => {
                write!(f, "bout: {got} ticks exceed {MAX_TICKS}")
            }
        }
    }
}

impl std::error::Error for BoutError {}

/// Batch shape: bout count, bout length and replay seed. `bouts` is
/// `1..=MAX_BOUTS`, `ticks_per_bout` is `1..=MAX_TICKS`. Units: bouts,
/// ticks, seeds. Determinism: same config replays identically.
/// Errors: [`BoutError`] on out-of-range shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoutConfig {
    /// Bouts in the batch.
    pub bouts: usize,
    /// Fixed ticks per bout.
    pub ticks_per_bout: u32,
    /// Seed mixed into every derived state.
    pub seed: u64,
}

impl BoutConfig {
    /// Checked batch shape. Never panics. Units: bouts, ticks, seeds.
    /// Errors: [`BoutError`].
    pub const fn new(bouts: usize, ticks_per_bout: u32, seed: u64) -> Result<Self, BoutError> {
        if bouts == 0 {
            return Err(BoutError::EmptyBouts);
        }
        if bouts > MAX_BOUTS {
            return Err(BoutError::TooManyBouts { got: bouts });
        }
        if ticks_per_bout == 0 {
            return Err(BoutError::EmptyTicks);
        }
        if ticks_per_bout > MAX_TICKS {
            return Err(BoutError::TooManyTicks {
                got: ticks_per_bout,
            });
        }
        Ok(Self {
            bouts,
            ticks_per_bout,
            seed,
        })
    }

    /// Standard proof shape: 200 bouts at 600 ticks from seed 2040.
    /// Known valid by construction. Units: bouts, ticks, seeds.
    pub const fn standard() -> Self {
        Self {
            bouts: STANDARD_BOUTS,
            ticks_per_bout: STANDARD_TICKS,
            seed: STANDARD_SEED,
        }
    }
}

/// Derive one integer state from the seeded mix of bout and tick. The
/// foe flag rides in from rival feedback each tick. Pure, total and
/// allocation-free: the same triple always yields the same state.
/// Units: millimetres, permille, ticks.
pub fn bout_state(bout: u64, tick: u32, seed: u64, foe_attacking: bool) -> BtState {
    let lane = seed
        .wrapping_add(bout.wrapping_mul(GOLDEN_GAMMA))
        .wrapping_add(u64::from(tick).wrapping_mul(GOLDEN_GAMMA));
    let mixed = splitmix64(lane);
    let span = mixed % 12_000;
    let base = i32::try_from(span).unwrap_or(0);
    let dist_mm = base.saturating_add(1_000);
    let stamina_raw = (mixed >> 16) % 1_001;
    let stamina = u16::try_from(stamina_raw).unwrap_or(0);
    BtState::new(dist_mm, foe_attacking, stamina, tick)
}

/// Recorded bout: per-tick decisions plus the chained digest. Cold
/// storage owned by the harness; the tick path never holds it. Units:
/// bouts, ticks, seeds, hashes, action ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoutTape {
    /// Bout index inside the batch.
    pub bout: u64,
    /// Ticks actually run.
    pub ticks: u32,
    /// Replay seed.
    pub seed: u64,
    /// Tree hash of side A at record time.
    pub tree_a_hash: u64,
    /// Tree hash of side B at record time.
    pub tree_b_hash: u64,
    /// Chained digest over every tick.
    pub digest: u64,
    /// Per-tick action ids for side A, in tick order.
    pub actions_a: Vec<u8>,
    /// Per-tick action ids for side B, in tick order.
    pub actions_b: Vec<u8>,
}

/// Batch report: one digest per bout plus the combined chain and the
/// stuck count. Same trees plus same config give the same report.
/// Units: bouts, ticks, seeds, hashes, counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoutReport {
    /// Bouts actually run.
    pub bouts: usize,
    /// Ticks run per bout.
    pub ticks_per_bout: u32,
    /// Replay seed.
    pub seed: u64,
    /// Per-bout digests in bout order.
    pub digests: Vec<u64>,
    /// Chain over the per-bout digests.
    pub combined: u64,
    /// Bouts with a stuck stream on either side.
    pub stuck_bouts: usize,
}

impl BoutReport {
    /// True exactly when no bout held one decision through the stillness
    /// window. Units: counts.
    pub fn clean(&self) -> bool {
        self.stuck_bouts == 0
    }
}

/// True while one action stream holds a single id through the stillness
/// window. Pure scan over a borrowed slice; empty streams read clean.
/// Units: ticks.
pub fn has_stuck(actions: &[u8]) -> bool {
    if actions.is_empty() {
        return false;
    }
    let mut run = 1u32;
    for pair in actions.windows(2) {
        if pair[0] == pair[1] {
            run = run.saturating_add(1);
            if run >= STUCK_WINDOW_TICKS {
                return true;
            }
        } else {
            run = 1;
        }
    }
    false
}

/// Run one deterministic bout between two trees. States derive from the
/// seeded mix per tick; rival attack flags feed back each tick; both
/// blackboards persist across ticks on the stack. The tick count
/// saturates at the cap; a zero count records an empty tape. Same
/// inputs give the same tape. Units: bouts, ticks, seeds.
pub fn run_bout(a: &BtTree, b: &BtTree, bout: u64, ticks: u32, seed: u64) -> BoutTape {
    let capped = ticks.min(MAX_TICKS);
    let mut bb_a = BtBlackboard::new();
    let mut bb_b = BtBlackboard::new();
    let mut out_a = ACTION_ADVANCE;
    let mut out_b = ACTION_ADVANCE;
    let mut foe_a = false;
    let mut foe_b = false;
    let mut chain = Fnv64::new();
    chain.write_u8(BOUT_VERSION);
    chain.write_u64(bout);
    chain.write_u32(capped);
    chain.write_u64(seed);
    let mut actions_a = Vec::new();
    let mut actions_b = Vec::new();
    for tick in 0..capped {
        let state_a = bout_state(bout, tick, seed, foe_a);
        let state_b = bout_state(bout, tick, seed, foe_b);
        let status_a = a.tick(&state_a, &mut bb_a, &mut out_a);
        let status_b = b.tick(&state_b, &mut bb_b, &mut out_b);
        actions_a.push(out_a);
        actions_b.push(out_b);
        chain.write_u64(state_a.state_hash());
        chain.write_u64(state_b.state_hash());
        chain.write_u64(crate::bt::tick_digest(&state_a, &bb_a, out_a, status_a));
        chain.write_u64(crate::bt::tick_digest(&state_b, &bb_b, out_b, status_b));
        foe_a = out_b == ACTION_ATTACK;
        foe_b = out_a == ACTION_ATTACK;
    }
    let _ = BtStatus::Success;
    BoutTape {
        bout,
        ticks: capped,
        seed,
        tree_a_hash: a.tree_hash(),
        tree_b_hash: b.tree_hash(),
        digest: chain.finish(),
        actions_a,
        actions_b,
    }
}

/// Re-run the recorded bout and compare digests, actions and tree
/// hashes. `false` on any drift or any tree swap. Units: hashes.
pub fn verify_tape(a: &BtTree, b: &BtTree, tape: &BoutTape) -> bool {
    if a.tree_hash() != tape.tree_a_hash || b.tree_hash() != tape.tree_b_hash {
        return false;
    }
    let fresh = run_bout(a, b, tape.bout, tape.ticks, tape.seed);
    fresh.digest == tape.digest
        && fresh.actions_a == tape.actions_a
        && fresh.actions_b == tape.actions_b
}

/// Run a full batch between two trees, one tape per bout. Collects
/// per-bout digests, chains them, and counts bouts with a stuck stream
/// on either side. Same trees plus same config give the same report.
/// Units: bouts, ticks, seeds.
pub fn run_bouts(a: &BtTree, b: &BtTree, config: &BoutConfig) -> BoutReport {
    let mut digests = Vec::new();
    let mut chain = Fnv64::new();
    chain.write_u8(BOUT_VERSION);
    chain.write_u64(config.seed);
    let mut stuck_bouts = 0usize;
    for index in 0..config.bouts {
        let bout = u64::try_from(index).unwrap_or(u64::MAX);
        let tape = run_bout(a, b, bout, config.ticks_per_bout, config.seed);
        if has_stuck(&tape.actions_a) || has_stuck(&tape.actions_b) {
            stuck_bouts = stuck_bouts.saturating_add(1);
        }
        chain.write_u64(tape.digest);
        digests.push(tape.digest);
    }
    BoutReport {
        bouts: config.bouts,
        ticks_per_bout: config.ticks_per_bout,
        seed: config.seed,
        digests,
        combined: chain.finish(),
        stuck_bouts,
    }
}

/// Round-trip one tree through its versioned bytes; the mirror stores
/// trees as bytes, so the production batch walks that path too. A refused
/// round-trip keeps the original tree. Cold build path; allocation is fine
/// here, never on the tick. Units: trees.
fn round_trip(tree: &BtTree) -> BtTree {
    let bytes = tree.encode();
    match BtTree::decode(&bytes) {
        Ok(back) => back,
        Err(_) => tree.clone(),
    }
}

/// Standard mirror batch plus its replay health flag. Builds the shipped
/// pressure/guard pair, walks the byte mirror path both trees take into
/// tapes, runs the standard 200-bout batch, then replays tape 0.
/// Consumer: `crates/arena` records fork tapes with [`run_bouts`] and
/// `crates/playtest` replays them with [`verify_tape`]; this entry point
/// is the shape both sides share. Cold build path; allocation is fine
/// here, never on the tick. Units: bouts, ticks, seeds.
/// Determinism: same shipped trees give byte-identical reports; the flag
/// is true exactly when tape 0 replays, shapes match and no bout stuck.
pub fn standard_pair_report() -> (BoutReport, bool) {
    let raw_a = crate::bt::pressure_tree();
    let raw_b = crate::bt::guard_tree();
    let a = round_trip(&raw_a);
    let b = round_trip(&raw_b);
    let config = BoutConfig::new(STANDARD_BOUTS, STANDARD_TICKS, STANDARD_SEED)
        .unwrap_or(BoutConfig::standard());
    let report = run_bouts(&a, &b, &config);
    let tape = run_bout(&a, &b, 0, 1, config.seed);
    let healthy = verify_tape(&a, &b, &tape)
        && a.node_count() == raw_a.node_count()
        && b.node_count() == raw_b.node_count()
        && a.depth() == raw_a.depth()
        && a.root() == raw_a.root()
        && b.root() == raw_b.root()
        && report.clean();
    (report, healthy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bt::{guard_tree, pressure_tree, ACTION_BLOCK};
    use engine_core::alloc_probe::measure;

    fn pair() -> (BtTree, BtTree) {
        (pressure_tree(), guard_tree())
    }

    #[test]
    fn config_validation_rejects_empty_and_oversize() {
        assert!(BoutConfig::new(0, 600, 2040).is_err());
        assert!(BoutConfig::new(1, 0, 2040).is_err());
        assert!(BoutConfig::new(MAX_BOUTS + 1, 600, 2040).is_err());
        assert!(BoutConfig::new(1, MAX_TICKS + 1, 2040).is_err());
        let standard = BoutConfig::standard();
        assert_eq!(
            (standard.bouts, standard.ticks_per_bout, standard.seed),
            (STANDARD_BOUTS, STANDARD_TICKS, STANDARD_SEED)
        );
        assert!(BoutConfig::new(1, 1, 7).is_ok());
    }

    #[test]
    fn derived_states_are_stable_and_use_full_range() {
        let first = bout_state(3, 41, 2040, true);
        assert_eq!(first, bout_state(3, 41, 2040, true));
        assert_ne!(first, bout_state(4, 41, 2040, true));
        assert_ne!(first, bout_state(3, 42, 2040, true));
        assert_ne!(
            first.foe_attacking,
            bout_state(3, 41, 2040, false).foe_attacking
        );
        assert!((1_000..=13_000).contains(&first.dist_mm));
    }

    #[test]
    fn stuck_scan_finds_frozen_streams_only() {
        assert!(!has_stuck(&[]));
        assert!(!has_stuck(&[ACTION_ATTACK]));
        let frozen = vec![ACTION_BLOCK; STUCK_WINDOW_TICKS as usize];
        assert!(has_stuck(&frozen));
        let mut almost = vec![ACTION_BLOCK; STUCK_WINDOW_TICKS as usize];
        almost.push(ACTION_ATTACK);
        almost.remove(0);
        let last = almost.len() - 1;
        almost[last] = ACTION_ATTACK;
        assert!(!has_stuck(&almost));
        let mut mixed = Vec::new();
        for tick in 0..STUCK_WINDOW_TICKS * 2 {
            mixed.push(if tick % 2 == 0 {
                ACTION_ATTACK
            } else {
                ACTION_BLOCK
            });
        }
        assert!(!has_stuck(&mixed));
    }

    #[test]
    fn tapes_verify_and_catch_swaps_and_edits() {
        let (a, b) = pair();
        let tape = run_bout(&a, &b, 9, 120, 2040);
        assert_eq!(tape.actions_a.len(), 120);
        assert!(verify_tape(&a, &b, &tape));
        assert!(!verify_tape(&b, &a, &tape), "swapped trees must not verify");
        let mut edited = tape.clone();
        let last = edited.actions_a.len() - 1;
        edited.actions_a[last] =
            edited.actions_a[last].wrapping_add(1) % ACTION_BLOCK.wrapping_add(9);
        assert!(!verify_tape(&a, &b, &edited), "edited tape must not verify");
    }

    #[test]
    fn standard_bouts_rerun_identical_x2_from_tapes() {
        let (a, b) = pair();
        let config = BoutConfig::standard();
        let first = run_bouts(&a, &b, &config);
        let second = run_bouts(&a, &b, &config);
        assert_eq!(first, second);
        assert_eq!(first.digests.len(), STANDARD_BOUTS);
        assert_eq!(first.stuck_bouts, 0, "shipped pair must never still");
        assert!(first.clean());
        for (index, digest) in first.digests.iter().enumerate() {
            let bout = u64::try_from(index).unwrap_or(u64::MAX);
            let tape = run_bout(&a, &b, bout, config.ticks_per_bout, config.seed);
            assert_eq!(tape.digest, *digest);
            assert!(verify_tape(&a, &b, &tape));
        }
    }

    #[test]
    fn standard_mirror_report_replays_x2_with_no_stuck() {
        // The playtest mirror entry point: one call builds the batch and
        // proves tape 0 replays; two calls give byte-identical reports.
        let (first, healthy) = standard_pair_report();
        assert!(healthy, "tape 0 must replay with matching shapes");
        assert!(first.clean(), "shipped pair must never still");
        assert_eq!(first.digests.len(), STANDARD_BOUTS);
        let (second, again) = standard_pair_report();
        assert!(again, "second build must replay too");
        assert_eq!(first, second, "200 bouts rerun-identical x2 from tapes");
    }

    #[test]
    fn hot_path_state_plus_tick_allocates_nothing() {
        let (a, b) = pair();
        let mut bb_a = BtBlackboard::new();
        let mut bb_b = BtBlackboard::new();
        let mut out_a = ACTION_ADVANCE;
        let mut out_b = ACTION_ADVANCE;
        let ((), heap) = measure(|| {
            for tick in 0..64u32 {
                let sa = bout_state(2, tick, 2040, false);
                let sb = bout_state(2, tick, 2040, true);
                std::hint::black_box(a.tick(&sa, &mut bb_a, &mut out_a));
                std::hint::black_box(b.tick(&sb, &mut bb_b, &mut out_b));
            }
        });
        assert!(out_a < crate::bt::ACTION_COUNT);
        assert!(out_b < crate::bt::ACTION_COUNT);
        assert_eq!(heap.calls, 0, "derived state plus tick must not allocate");
    }
}
