//! crowd_scale: thousand-agent crowd tape over the LOD tiers.
//!
//! Owner: `engine-anim`; status: scale proof beside the frozen tier table.
//! Consumer: the render-side impostor pass reads [`FarImpostorSink`]; the
//! perf gate reads the posted cost table. Units: meters, screen fraction
//! `0..=1`, nanoseconds per tick. Determinism: seeded integer draws plus
//! `total_cmp` rank order only, ties by agent index, no clock. Errors:
//! [`CrowdError`]. Budget: slices in, slice out, no heap per call.

use super::lod::{tier_for, Tier, FULL_RATE_NEAREST};
use engine_core::{Fnv64, Rng};

/// Agents per crowd tape; bounds the stack rank.
pub const MAX_CROWD: usize = 1000;
/// Canonical crowd seed; reseed paths derive from it by xor folds.
pub const CROWD_SEED: u64 = 2040;
/// Near end of a crowd lane in meters.
pub const DIST_MIN_M: f32 = 2.0;
/// Far end of a crowd lane in meters; a lane past it wraps back to near.
pub const DIST_MAX_M: f32 = 122.0;
/// Canonical per-tick crowd stride in meters: a live crowd strafes this far
/// along its lane each tick, so the nearest set moves.
pub const CROWD_STRIDE_M: f32 = 0.5;
/// Modeled full-rate eval cost per agent per tick, in nanoseconds.
pub const COST_FULL_NS: u64 = 1500;
/// Amortized half-rate cost: one full eval every 2 ticks.
pub const COST_HALF_NS: u64 = 750;
/// Amortized quarter-rate cost: one full eval every 4 ticks.
pub const COST_QUARTER_NS: u64 = 375;
/// Amortized far-ring handoff cost: one impostor push every 8 ticks.
pub const COST_FAR_NS: u64 = 20;
/// Reference-CPU p99 ceiling for the thousand-agent tick, in nanoseconds.
pub const BUDGET_P99_NS: u64 = 2_000_000;

/// Bounded crowd-tape failures: typed, never panics.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CrowdError {
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
    /// Distance below zero; value carried.
    NegativeDistance(f32),
    /// Screen fraction outside `0..=1`; value carried.
    BadScreen(f32),
    /// Lane count mismatch; expected plus got.
    LengthMismatch { expected: usize, got: usize },
    /// Entry count past [`MAX_CROWD`]; value carried.
    TooManyAgents(usize),
}

impl core::fmt::Display for CrowdError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CrowdError::NonFinite(lane) => write!(f, "crowd: non-finite float in {lane}"),
            CrowdError::NegativeDistance(v) => write!(f, "crowd: distance {v} below zero"),
            CrowdError::BadScreen(v) => write!(f, "crowd: screen {v} outside 0..=1"),
            CrowdError::LengthMismatch { expected, got } => {
                write!(f, "crowd: lane count {got} differs from {expected}")
            }
            CrowdError::TooManyAgents(n) => {
                write!(f, "crowd: {n} agents past {MAX_CROWD}")
            }
        }
    }
}

impl std::error::Error for CrowdError {}

/// Far-ring handoff: the CPU contract the render-side impostor pass
/// consumes. The render side owns the impostor; this side only routes
/// which agents hand off, with their distance and screen fraction.
pub trait FarImpostorSink {
    /// Accept one far agent by index, with its distance and screen fraction.
    fn push_far(&mut self, agent: usize, distance_m: f32, screen_frac: f32);
}

/// Null far-ring sink: counts handed agents, stores nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NullSink {
    /// Agents handed off so far.
    pub handed: u64,
}

impl NullSink {
    /// Empty sink.
    pub fn new() -> Self {
        Self { handed: 0 }
    }
}

impl FarImpostorSink for NullSink {
    fn push_far(&mut self, _agent: usize, _distance_m: f32, _screen_frac: f32) {
        self.handed += 1;
    }
}

/// Seeded thousand-agent crowd tape: distances plus screen fractions.
/// Built once at setup; the per-tick path only reads these lanes.
#[derive(Clone, Debug, PartialEq)]
pub struct CrowdTape {
    /// Distance per agent in meters, `2.0..122.0`.
    pub distances: [f32; MAX_CROWD],
    /// Screen fraction per agent, `0.0..0.03`, skewed small.
    pub screens: [f32; MAX_CROWD],
}

impl CrowdTape {
    /// Deterministic tape from integer draws: distances uniform over
    /// `2..122` m, screens square-skewed under `0.03` so only near
    /// agents read big. Same seed gives the same lanes twice.
    pub fn seeded(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let mut distances = [0.0f32; MAX_CROWD];
        let mut screens = [0.0f32; MAX_CROWD];
        for (d, s) in distances.iter_mut().zip(screens.iter_mut()) {
            let d16 = u16::try_from(rng.next_u32() % 12_000).unwrap_or(u16::MAX);
            *d = DIST_MIN_M + f32::from(d16) / 100.0;
            let w16 = u16::try_from(rng.next_u32() % 10_000).unwrap_or(u16::MAX);
            let frac = f32::from(w16) / 10_000.0;
            *s = frac * frac * 0.03;
        }
        Self { distances, screens }
    }

    /// Move every agent `stride_m` along its lane; a distance that reaches
    /// [`DIST_MAX_M`] wraps back by the lane length, so the nearest set
    /// changes over time and a ranking run sees a moving crowd. Plain
    /// arithmetic over the lane, no clock and no draw: the same tape and
    /// stride give the same lanes twice. A non-finite stride poisons the
    /// lanes, which [`assign_crowd_tiers`] then refuses as
    /// [`CrowdError::NonFinite`].
    pub fn advance(&mut self, stride_m: f32) {
        let lane_m = DIST_MAX_M - DIST_MIN_M;
        for d in self.distances.iter_mut() {
            *d += stride_m;
            if *d >= DIST_MAX_M {
                *d -= lane_m;
            }
        }
    }
}

/// Validate the crowd lanes: equal lengths within [`MAX_CROWD`], every
/// distance finite and non-negative, every screen in `0..=1`.
fn check_lanes(distances: &[f32], screens: &[f32]) -> Result<(), CrowdError> {
    if distances.len() != screens.len() {
        return Err(CrowdError::LengthMismatch {
            expected: distances.len(),
            got: screens.len(),
        });
    }
    if distances.len() > MAX_CROWD {
        return Err(CrowdError::TooManyAgents(distances.len()));
    }
    for (&d, &s) in distances.iter().zip(screens.iter()) {
        if !d.is_finite() {
            return Err(CrowdError::NonFinite("distances_m"));
        }
        if !s.is_finite() {
            return Err(CrowdError::NonFinite("screen_fracs"));
        }
        if d < 0.0 {
            return Err(CrowdError::NegativeDistance(d));
        }
        if !(0.0..=1.0).contains(&s) {
            return Err(CrowdError::BadScreen(s));
        }
    }
    Ok(())
}

/// Fill `out` with one tier per agent from the frozen tier table, then
/// force the [`FULL_RATE_NEAREST`] nearest to [`Tier::Full`]. The rank
/// is global across all thousand agents, not per chunk. Slices in,
/// slice out: no heap. Ties break by agent index, so one tape assigns
/// identically twice.
pub fn assign_crowd_tiers(
    distances: &[f32],
    screens: &[f32],
    out: &mut [Tier],
) -> Result<(), CrowdError> {
    check_lanes(distances, screens)?;
    if out.len() != distances.len() {
        return Err(CrowdError::LengthMismatch {
            expected: distances.len(),
            got: out.len(),
        });
    }
    for (i, slot) in out.iter_mut().enumerate() {
        // Validated above, so the lookup cannot fail.
        match tier_for(distances[i], screens[i]) {
            Ok(t) => *slot = t,
            Err(_) => *slot = Tier::Far,
        }
    }
    // Stack rank of agent indices by distance, ties by index.
    let mut order = [0usize; MAX_CROWD];
    for (rank, slot) in order.iter_mut().take(distances.len()).enumerate() {
        *slot = rank;
    }
    let (ranked, _) = order.split_at_mut(distances.len());
    ranked.sort_unstable_by(|&a, &b| {
        distances[a]
            .total_cmp(&distances[b])
            .then_with(|| a.cmp(&b))
    });
    let keep = FULL_RATE_NEAREST.min(out.len());
    for &agent in ranked.iter().take(keep) {
        out[agent] = Tier::Full;
    }
    Ok(())
}

/// Hand every [`Tier::Far`] agent to `sink`, in agent order. Validates
/// all lanes before touching the sink, so a refusal hands off nothing.
/// Returns the handed count.
pub fn collect_far(
    distances: &[f32],
    screens: &[f32],
    tiers: &[Tier],
    sink: &mut impl FarImpostorSink,
) -> Result<u64, CrowdError> {
    check_lanes(distances, screens)?;
    if tiers.len() != distances.len() {
        return Err(CrowdError::LengthMismatch {
            expected: distances.len(),
            got: tiers.len(),
        });
    }
    let mut handed = 0u64;
    for (i, &tier) in tiers.iter().enumerate() {
        if tier == Tier::Far {
            sink.push_far(i, distances[i], screens[i]);
            handed += 1;
        }
    }
    Ok(handed)
}

/// Per-tier agent counts for one assigned tape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TierCounts {
    /// Agents at [`Tier::Full`].
    pub full: u64,
    /// Agents at [`Tier::Half`].
    pub half: u64,
    /// Agents at [`Tier::Quarter`].
    pub quarter: u64,
    /// Agents at [`Tier::Far`].
    pub far: u64,
}

impl TierCounts {
    /// No agent in any tier: the value a refused tick folds.
    pub const EMPTY: Self = Self {
        full: 0,
        half: 0,
        quarter: 0,
        far: 0,
    };
}

/// Count one tier per lane. Infallible by design: counts any lane slice.
pub fn count_tiers(tiers: &[Tier]) -> TierCounts {
    let mut counts = TierCounts {
        full: 0,
        half: 0,
        quarter: 0,
        far: 0,
    };
    for &tier in tiers {
        match tier {
            Tier::Full => counts.full += 1,
            Tier::Half => counts.half += 1,
            Tier::Quarter => counts.quarter += 1,
            Tier::Far => counts.far += 1,
        }
    }
    counts
}

/// Modeled tick cost in nanoseconds: per-tier counts times the posted
/// amortized tier costs. Deterministic; wall clock never enters.
pub fn estimate_tick_cost_ns(counts: &TierCounts) -> u64 {
    counts.full * COST_FULL_NS
        + counts.half * COST_HALF_NS
        + counts.quarter * COST_QUARTER_NS
        + counts.far * COST_FAR_NS
}

/// Canonical hash over both lanes: length first, then every lane by bit
/// pattern, little-endian through the shared hasher. Same tape hashes
/// identically twice; wall clock never enters.
pub fn tape_hash(distances: &[f32], screens: &[f32]) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_u64(u64::try_from(distances.len()).unwrap_or(u64::MAX));
    for &d in distances {
        hasher.write_f32(d);
    }
    hasher.write_u64(u64::try_from(screens.len()).unwrap_or(u64::MAX));
    for &s in screens {
        hasher.write_f32(s);
    }
    hasher.finish()
}

/// One crowd tick verdict: per-tier counts, modeled cost, far handoff
/// and tape hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrowdReport {
    /// Per-tier agent counts.
    pub counts: TierCounts,
    /// Modeled tick cost in nanoseconds.
    pub cost_ns: u64,
    /// Far agents handed to the impostor sink.
    pub far_handed: u64,
    /// Canonical hash of the tape lanes.
    pub hash: u64,
}

/// Assign `tape` into `tiers`, then count tiers, post the modeled cost
/// and hand the far ring to a null sink. The single production caller
/// of the assign, count, cost and handoff path.
pub fn crowd_tick_report(tape: &CrowdTape, tiers: &mut [Tier]) -> Result<CrowdReport, CrowdError> {
    assign_crowd_tiers(&tape.distances, &tape.screens, tiers)?;
    let counts = count_tiers(tiers);
    let cost_ns = estimate_tick_cost_ns(&counts);
    let mut sink = NullSink::new();
    let far_handed = collect_far(&tape.distances, &tape.screens, tiers, &mut sink)?;
    let hash = tape_hash(&tape.distances, &tape.screens);
    Ok(CrowdReport {
        counts,
        cost_ns,
        far_handed,
        hash,
    })
}

/// Canonical thousand-agent verdict for `seed`: builds the tape, then
/// runs the tick report. The single production caller of [`CrowdTape::seeded`].
pub fn seeded_report(seed: u64) -> Result<CrowdReport, CrowdError> {
    let tape = CrowdTape::seeded(seed);
    let mut tiers = [Tier::Far; MAX_CROWD];
    crowd_tick_report(&tape, &mut tiers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe;

    /// Seed tweak that provably differs (high bits flip, never the legacy
    /// low-bit forcing).
    const SEED_TWEAK: u64 = 0x9E37_79B9;

    /// Indices of the `n` nearest agents by distance, ties by index: the
    /// same rule the assign path uses, recomputed here as the oracle.
    fn nearest_indices(distances: &[f32], n: usize) -> Vec<usize> {
        let mut order: Vec<usize> = (0..distances.len()).collect();
        order.sort_by(|&a, &b| {
            distances[a]
                .total_cmp(&distances[b])
                .then_with(|| a.cmp(&b))
        });
        order.truncate(n);
        order
    }

    #[test]
    fn thousand_agents_assign_green() {
        let tape = CrowdTape::seeded(CROWD_SEED);
        assert_eq!(tape.distances.len(), MAX_CROWD);
        assert_eq!(tape.screens.len(), MAX_CROWD);
        let mut tiers = [Tier::Far; MAX_CROWD];
        assign_crowd_tiers(&tape.distances, &tape.screens, &mut tiers)
            .expect("seeded tape assigns");
        assert_eq!(tiers.len(), MAX_CROWD);
    }

    #[test]
    fn eight_nearest_match_full_rate_poses() {
        // The accept bar: the 8 nearest agents eval every tick with IK and
        // springs on, exactly like full-rate poses, whatever their lanes read.
        let tape = CrowdTape::seeded(CROWD_SEED);
        let mut tiers = [Tier::Far; MAX_CROWD];
        assign_crowd_tiers(&tape.distances, &tape.screens, &mut tiers)
            .expect("seeded tape assigns");
        let nearest = nearest_indices(&tape.distances, FULL_RATE_NEAREST);
        assert_eq!(nearest.len(), FULL_RATE_NEAREST);
        for agent in nearest {
            assert_eq!(
                tiers[agent],
                Tier::Full,
                "nearest agent {agent} holds full rate"
            );
            assert!(
                tiers[agent].ik_on() && tiers[agent].springs_on(),
                "nearest agent {agent} keeps IK and springs"
            );
            assert!(
                tiers[agent].updates_this_tick(7, agent),
                "nearest agent {agent} evals every tick"
            );
        }
    }

    #[test]
    fn seeded_tape_repeats_x2_and_reseed_moves() {
        let (first, second) = (CrowdTape::seeded(CROWD_SEED), CrowdTape::seeded(CROWD_SEED));
        assert_eq!(first.distances, second.distances, "distances repeat");
        assert_eq!(first.screens, second.screens, "screens repeat");
        assert_eq!(
            tape_hash(&first.distances, &first.screens),
            tape_hash(&second.distances, &second.screens),
            "hash repeats"
        );
        let mut tiers = [Tier::Far; MAX_CROWD];
        let mut again = [Tier::Far; MAX_CROWD];
        assign_crowd_tiers(&first.distances, &first.screens, &mut tiers).expect("first assigns");
        assign_crowd_tiers(&second.distances, &second.screens, &mut again).expect("second assigns");
        assert_eq!(tiers, again, "assign repeats byte-identically");
        let moved = CrowdTape::seeded(CROWD_SEED ^ SEED_TWEAK);
        assert_ne!(
            tape_hash(&moved.distances, &moved.screens),
            tape_hash(&first.distances, &first.screens),
            "reseed moves the tape"
        );
    }

    #[test]
    fn advance_strafes_every_lane_and_a_full_lap_returns() {
        // Ticks for one full 120 m lane at the canonical stride.
        const LAP_TICKS: u32 = 240;
        assert_eq!(
            CROWD_STRIDE_M * LAP_TICKS as f32,
            DIST_MAX_M - DIST_MIN_M,
            "one lap is the whole lane"
        );
        let mut tape = CrowdTape::seeded(CROWD_SEED);
        let start = tape.distances;
        let lane_m = DIST_MAX_M - DIST_MIN_M;
        tape.advance(CROWD_STRIDE_M);
        let mut wrapped = 0usize;
        for (i, &d0) in start.iter().enumerate() {
            let step = tape.distances[i] - d0;
            if (step - CROWD_STRIDE_M).abs() < 1e-3 {
                continue;
            }
            assert!(
                (step + lane_m - CROWD_STRIDE_M).abs() < 1e-3,
                "agent {i} moved {step} m, neither one stride nor a wrap"
            );
            wrapped += 1;
        }
        assert!(
            wrapped > 0,
            "a thousand-agent tape has lanes at the far end"
        );
        // A full lap leaves every lane where it started, wrapped exactly once.
        // Additions round at f32, so the round trip is approximate.
        let mut lap = tape.clone();
        for _ in 1..LAP_TICKS {
            lap.advance(CROWD_STRIDE_M);
        }
        let worst_drift = start
            .iter()
            .zip(lap.distances.iter())
            .fold(0.0f32, |worst, (&d0, &d1)| worst.max((d1 - d0).abs()));
        assert!(
            worst_drift < 1e-3,
            "a full lap returns every lane, worst drift {worst_drift} m"
        );
        // A non-finite stride poisons the lane, and the assign refuses it.
        lap.advance(f32::NAN);
        let mut out = [Tier::Far; MAX_CROWD];
        assert_eq!(
            assign_crowd_tiers(&lap.distances, &lap.screens, &mut out),
            Err(CrowdError::NonFinite("distances_m")),
            "a NaN stride is refused, not ranked"
        );
    }

    /// Recording sink: the impostor contract's test double.
    struct VecSink {
        seen: Vec<usize>,
    }

    impl FarImpostorSink for VecSink {
        fn push_far(&mut self, agent: usize, _distance_m: f32, _screen_frac: f32) {
            self.seen.push(agent);
        }
    }

    #[test]
    fn far_ring_hands_to_impostor_sink() {
        let tape = CrowdTape::seeded(CROWD_SEED);
        let mut tiers = [Tier::Far; MAX_CROWD];
        assign_crowd_tiers(&tape.distances, &tape.screens, &mut tiers)
            .expect("seeded tape assigns");
        let mut sink = VecSink { seen: Vec::new() };
        let handed =
            collect_far(&tape.distances, &tape.screens, &tiers, &mut sink).expect("handoff holds");
        assert!(handed > 0, "a thousand-agent tape has a far ring");
        assert_eq!(
            handed,
            u64::try_from(sink.seen.len()).unwrap_or(u64::MAX),
            "every handoff lands"
        );
        let far_count =
            u64::try_from(tiers.iter().filter(|&&t| t == Tier::Far).count()).unwrap_or(u64::MAX);
        assert_eq!(handed, far_count, "handoff matches the far count");
        for agent in &sink.seen {
            assert_eq!(
                tiers[*agent],
                Tier::Far,
                "handed agent {agent} holds the far tier"
            );
        }
        // Agent order is stable: handoffs arrive in index order.
        let mut ordered = sink.seen.clone();
        ordered.sort_unstable();
        assert_eq!(sink.seen, ordered, "handoff order is stable");
    }

    #[test]
    fn handoff_refuses_mismatch_without_touching_sink() {
        let tape = CrowdTape::seeded(CROWD_SEED);
        let mut tiers = [Tier::Far; MAX_CROWD];
        assign_crowd_tiers(&tape.distances, &tape.screens, &mut tiers)
            .expect("seeded tape assigns");
        let short = [Tier::Far; 16];
        let mut sink = NullSink::new();
        assert_eq!(
            collect_far(&tape.distances, &tape.screens, &short, &mut sink),
            Err(CrowdError::LengthMismatch {
                expected: MAX_CROWD,
                got: 16
            }),
            "short tiers refuse"
        );
        assert_eq!(sink.handed, 0, "refusal hands off nothing");
    }

    #[test]
    fn rejects_bad_input() {
        let mut out = [Tier::Far; 2];
        let bad = [10.0f32, f32::NAN];
        let screens = [0.01f32, 0.01];
        assert_eq!(
            assign_crowd_tiers(&bad, &screens, &mut out),
            Err(CrowdError::NonFinite("distances_m")),
            "NaN distance refuses"
        );
        let neg = [10.0f32, -1.0];
        assert_eq!(
            assign_crowd_tiers(&neg, &screens, &mut out),
            Err(CrowdError::NegativeDistance(-1.0)),
            "negative distance refuses"
        );
        let wide = [10.0f32, 10.0];
        let wide_s = [0.01f32, 2.0];
        assert_eq!(
            assign_crowd_tiers(&wide, &wide_s, &mut out),
            Err(CrowdError::BadScreen(2.0)),
            "screen past 1 refuses"
        );
        let short_s = [0.01f32];
        assert_eq!(
            assign_crowd_tiers(&wide, &short_s, &mut out),
            Err(CrowdError::LengthMismatch {
                expected: 2,
                got: 1
            }),
            "lane mismatch refuses"
        );
        let big_d = [80.0f32; MAX_CROWD + 1];
        let big_s = [0.002f32; MAX_CROWD + 1];
        let mut big_out = [Tier::Far; MAX_CROWD + 1];
        assert_eq!(
            assign_crowd_tiers(&big_d, &big_s, &mut big_out),
            Err(CrowdError::TooManyAgents(MAX_CROWD + 1)),
            "1001 agents refuse"
        );
    }

    #[test]
    fn tick_path_allocates_nothing() {
        // Calibrate the probe first: a zero reading must mean zero.
        let (_, seen) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(seen.calls >= 1, "alloc probe is blind");
        let ((), allocs) = alloc_probe::measure(|| {
            let tape = CrowdTape::seeded(CROWD_SEED);
            let mut tiers = [Tier::Far; MAX_CROWD];
            assign_crowd_tiers(&tape.distances, &tape.screens, &mut tiers)
                .expect("seeded tape assigns");
            let counts = count_tiers(&tiers);
            let cost = estimate_tick_cost_ns(&counts);
            let mut sink = NullSink::new();
            let handed = collect_far(&tape.distances, &tape.screens, &tiers, &mut sink)
                .expect("handoff holds");
            std::hint::black_box((tiers[0], tiers[MAX_CROWD - 1], counts, cost, handed));
        });
        assert_eq!(allocs.calls, 0, "tick path allocates");
    }

    #[test]
    fn modeled_tick_cost_posts_per_tier_within_budget() {
        // Per-tier costs are posted, never averaged away: the amortization
        // relation pins strobe periods (2, 4, 8) against the full eval.
        assert_eq!(COST_HALF_NS * 2, COST_FULL_NS, "half amortizes period 2");
        assert_eq!(
            COST_QUARTER_NS * 4,
            COST_FULL_NS,
            "quarter amortizes period 4"
        );
        const {
            assert!(COST_FAR_NS < COST_QUARTER_NS, "far handoff is cheapest");
        }
        let first = seeded_report(CROWD_SEED).expect("seeded report holds");
        let second = seeded_report(CROWD_SEED).expect("report repeats");
        assert_eq!(first, second, "report repeats byte-identically");
        let total = first.counts.full + first.counts.half + first.counts.quarter + first.counts.far;
        assert_eq!(
            total,
            u64::try_from(MAX_CROWD).unwrap_or(u64::MAX),
            "counts cover every agent"
        );
        assert_eq!(
            first.far_handed, first.counts.far,
            "handoff matches far count"
        );
        assert!(
            first.cost_ns <= BUDGET_P99_NS,
            "modeled tick {} ns past {} ns",
            first.cost_ns,
            BUDGET_P99_NS
        );
        assert_eq!(
            first.hash,
            tape_hash(
                &CrowdTape::seeded(CROWD_SEED).distances,
                &CrowdTape::seeded(CROWD_SEED).screens
            ),
            "report pins the tape"
        );
    }

    /// Timed ticks in the p99 window: 2048 samples put the p99 at the 21st
    /// worst tick (sorted index `2048 * 99 / 100`).
    const P99_SAMPLES: usize = 2048;
    /// Ticks run and discarded before the timed window (first-touch, branch
    /// and cache warm).
    const WARMUP_TICKS: usize = 256;

    /// Reference-CPU p99 gate for the thousand-agent crowd tick, MEASURED
    /// with the wall clock (test-only; the tick itself reads no clock).
    ///
    /// Machine: 13th Gen Intel Core i5-13420H (8 cores / 12 threads, 2.1 GHz
    /// base), Windows 10 Home, this repo's PC.
    /// Method: the canonical seeded tape `CrowdTape::seeded(CROWD_SEED)`
    /// (seed `2040`, 1000 agents), advanced 0.5 m per tick and wrapped so the
    /// near set moves; [`WARMUP_TICKS`] ticks are discarded, then
    /// [`P99_SAMPLES`] ticks are timed one by one around
    /// [`crowd_tick_report`] (tier assign, per-tier count, modeled cost post,
    /// far handoff, tape digest). p99 is the 21st worst of the 2048 sorted
    /// samples. Run release, one thread:
    /// `cargo test -p engine-anim --lib crowd_scale --release -- --ignored
    /// --nocapture`.
    ///
    /// Scope: the crowd LOD tick. Per-agent pose eval is not in this crate's
    /// thousand-agent path (a pose table holds at most
    /// [`MAX_FIGHTERS`](crate::lod::MAX_FIGHTERS) fighters), so the per-tier
    /// eval constants stay modeled by
    /// `modeled_tick_cost_posts_per_tier_within_budget`.
    #[test]
    #[ignore = "timing: reference-CPU p99 gate, run with --ignored --release"]
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "wall clock in a test-only timing report, never in the tick"
    )]
    fn thousand_agent_tick_p99_within_budget() {
        let mut tape = CrowdTape::seeded(CROWD_SEED);
        let mut tiers = [Tier::Far; MAX_CROWD];
        for _ in 0..WARMUP_TICKS {
            tape.advance(CROWD_STRIDE_M);
            std::hint::black_box(crowd_tick_report(&tape, &mut tiers).expect("warmup tick"));
        }
        let mut samples = [0u32; P99_SAMPLES];
        for slot in samples.iter_mut() {
            tape.advance(CROWD_STRIDE_M);
            let start = std::time::Instant::now();
            let report = crowd_tick_report(&tape, &mut tiers).expect("timed tick");
            let ns = start.elapsed().as_nanos();
            *slot = u32::try_from(ns).unwrap_or(u32::MAX);
            std::hint::black_box(report);
        }
        samples.sort_unstable();
        let p50 = samples[P99_SAMPLES / 2];
        let p99 = samples[(P99_SAMPLES * 99) / 100];
        let worst = samples[P99_SAMPLES - 1];
        let report = crowd_tick_report(&tape, &mut tiers).expect("report holds");
        let counts = report.counts;
        println!(
            "crowd_scale: seed {CROWD_SEED} agents {MAX_CROWD} warmup {WARMUP_TICKS} samples {P99_SAMPLES}"
        );
        println!(
            "  tiers: full {} half {} quarter {} far {} (far handed {})",
            counts.full, counts.half, counts.quarter, counts.far, report.far_handed
        );
        println!(
            "  posted per-tier ns: full {COST_FULL_NS} half {COST_HALF_NS} quarter {COST_QUARTER_NS} far {COST_FAR_NS} -> modeled tick {} ns",
            estimate_tick_cost_ns(&counts)
        );
        println!(
            "  measured crowd tick: p50 {p50} ns p99 {p99} ns max {worst} ns (bar {BUDGET_P99_NS} ns)"
        );
        assert!(
            u64::from(p99) <= BUDGET_P99_NS,
            "1,000-agent crowd tick p99 {p99} ns past the {BUDGET_P99_NS} ns bar"
        );
    }
}
