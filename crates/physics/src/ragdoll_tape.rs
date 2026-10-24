//! ragdoll_tape: frozen-denominator baseline tape, seeded falls plus a clinch.
//!
//! Consumer: the contacts/groups fix, the CCD enable and the swept-cast
//! detector rows, plus the `ragdoll_gap` acceptance test and the
//! `tests/ragdoll_tape.rs` integration test. Data owner: this module owns
//! the frozen fixture (two four-bone chains over a floor box), the seeded
//! fall/clinch scripts and the `TapeReport` numbers. Units: meters,
//! seconds, ticks at `FIXED_DT`; hashes are
//! `u64` FNV digests, allocations are heap-call counts.
//!
//! The tape is read-only on the tick: it only calls the public
//! `Ragdoll` API (`spawn`, `set_animated`,
//! `blend_to_ragdoll`, `apply_hit`, `step`, position/hash reads) and never
//! touches simulation code, so the measurement itself cannot drift a hash
//! (the `ragdoll_gap` test re-runs the tape through the plain stepper and
//! demands the same digest). Determinism scope: same `TapeConfig` seed
//! and counts give the identical `TapeReport` on every platform; per-fall
//! streams split with `Rng::fork`, the clinch stream with
//! `Rng::fork_named`; the glue math is `+ - * /`, one `sqrt` per bone
//! pair distance and integer hashing. Errors: `TapeError` is typed;
//! invalid configs and ragdoll failures return, never panic.
//!
//! Budget: the baseline runs 50 falls of 360 ticks plus a 660-tick clinch
//! stepped by two fighters (19,320 fixed steps of at most 4 bodies each),
//! seconds on a desktop. The per-tick heap cost is posted in the report,
//! not required: the zero-alloc audit is a later row's job.
#![forbid(unsafe_code)]

use super::clinch::{deepest_overlap, part_members, Member};
use super::ragdoll::{BoneShape, Error as RagdollError, Ragdoll};
use super::rapier::FIXED_DT;
use engine_anim::contract::{Joint, LocalPose, Skeleton};
use engine_core::Rng;
use std::fmt::{Display, Formatter};

/// Pinned seed for the whole tape; per-fall streams fork from it.
pub const TAPE_SEED: u64 = 2040;
/// Frozen baseline fall count.
pub const FALLS: usize = 50;
/// Fixed steps per fall before get-up (5 s at 60 Hz: falls, hits, rests).
pub const FALL_STEPS: u32 = 300;
/// Blend-back window after each fall and after the clinch, in ticks.
/// Mirrors `powered::GETUP_BUDGET_TICKS`; the equality test below pins it.
pub const GETUP_STEPS: u32 = 60;
/// Fixed steps of the two-fighter clinch (10 s at 60 Hz).
pub const CLINCH_STEPS: u32 = 600;
/// Lateral spawn gap between the two clinch fighters, meters.
pub const CLINCH_GAP_M: f32 = 0.6;
/// Floor top plane height, meters (floor box `[-10..10] x [-1..0] x [-10..10]`).
pub const FLOOR_TOP_Y: f32 = 0.0;
/// Sanity cap on reported floor penetration, meters. Resting bones read
/// near zero (center-based probe); anything past this cap means tunneling,
/// which is the CCD enable row's to fix.
pub const PENETRATION_CAP_M: f32 = 0.30;
/// Capsule radius per fixture bone, meters.
pub const BONE_RADIUS: f32 = 0.15;
/// Capsule cylinder half-height per fixture bone, meters.
pub const BONE_HALF_HEIGHT: f32 = 0.2;
/// Spawn height of the fixture chain root, meters.
pub const ROOT_HEIGHT: f32 = 3.0;
/// Earliest tick a seeded hit may land (bones need airtime first).
pub const HIT_MIN_TICK: u32 = 30;
/// Ticks of margin kept after the latest seeded hit.
pub const HIT_TAIL_TICKS: u32 = 30;
/// Seeded hit impulse half-range, newton-seconds (matches the
/// powered-ragdoll hit scale).
pub const HIT_HALF_RANGE: f32 = 0.06;
/// Seeded lateral spawn offset half-range, meters.
pub const SPAWN_HALF_RANGE: f32 = 0.5;

/// Typed errors for the tape. Caller input never panics.
#[derive(Clone, Debug, PartialEq)]
pub enum TapeError {
    /// The underlying ragdoll call failed; the [`RagdollError`] names it.
    Ragdoll(RagdollError),
    /// `falls` must be at least 1.
    NoFalls,
    /// Step counts must clear the hit window (`fall_steps` past
    /// `HIT_MIN_TICK + HIT_TAIL_TICKS`, nonzero get-up and clinch).
    NoSteps,
}

impl Display for TapeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            TapeError::Ragdoll(e) => write!(f, "ragdoll failed: {e}"),
            TapeError::NoFalls => f.write_str("tape needs at least 1 fall"),
            TapeError::NoSteps => f.write_str("tape step counts miss the hit window"),
        }
    }
}

impl std::error::Error for TapeError {}

impl From<RagdollError> for TapeError {
    fn from(e: RagdollError) -> Self {
        TapeError::Ragdoll(e)
    }
}

/// Heap cost of one fixed step, in calls and bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepCost {
    /// Heap allocation calls during the step (growth by realloc counts).
    pub calls: u64,
    /// Bytes requested by those calls.
    pub bytes: u64,
}

impl StepCost {
    /// Zero cost: what the plain stepper reports (it never counts).
    pub const ZERO: Self = Self { calls: 0, bytes: 0 };
}

/// Wraps one fixed step so tests can count heap use without touching the
/// simulation. The plain runner uses a forwarder that reports
/// [`StepCost::ZERO`]; the acceptance test passes a probe sampler built on
/// `engine_core::alloc_probe` and the runner folds the per-step maxima
/// into the report.
pub trait StepSampler {
    /// Advance `ragdoll` by `dt` and return the step's heap cost.
    /// Errors: forwards [`TapeError::Ragdoll`] when the step rejects `dt`.
    fn step(&mut self, ragdoll: &mut Ragdoll, dt: f32) -> Result<StepCost, TapeError>;
}

/// Frozen tape configuration: seed plus step counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TapeConfig {
    /// Master seed; fall `i` forks stream `i`, the clinch forks `"clinch"`.
    pub seed: u64,
    /// Seeded falls to run (baseline: [`FALLS`]).
    pub falls: usize,
    /// Fixed steps per fall before get-up (baseline: [`FALL_STEPS`]).
    pub fall_steps: u32,
    /// Blend-back ticks after each fall and the clinch (baseline: [`GETUP_STEPS`]).
    pub getup_steps: u32,
    /// Fixed steps of the two-fighter clinch (baseline: [`CLINCH_STEPS`]).
    pub clinch_steps: u32,
}

impl TapeConfig {
    /// The frozen baseline fixture: 50 seeded falls plus the clinch.
    pub fn baseline() -> Self {
        Self {
            seed: TAPE_SEED,
            falls: FALLS,
            fall_steps: FALL_STEPS,
            getup_steps: GETUP_STEPS,
            clinch_steps: CLINCH_STEPS,
        }
    }

    /// Checked constructor for drivers (integration test and fix-row
    /// harnesses). Errors: [`TapeError::NoFalls`] for `falls == 0`,
    /// [`TapeError::NoSteps`] when the counts miss the hit window.
    pub fn new(
        seed: u64,
        falls: usize,
        fall_steps: u32,
        getup_steps: u32,
        clinch_steps: u32,
    ) -> Result<Self, TapeError> {
        let config = Self {
            seed,
            falls,
            fall_steps,
            getup_steps,
            clinch_steps,
        };
        config.validate()?;
        Ok(config)
    }

    /// Total `step` calls one run performs: one per fighter per tick, so
    /// the clinch counts double (two fighters). Deterministic in the config
    /// alone.
    pub fn ticks(&self) -> u64 {
        // `falls` is a small loop count (baseline 50); the saturating
        // widening matches the old cast exactly on every reachable config.
        u64::try_from(self.falls).unwrap_or(u64::MAX)
            * u64::from(self.fall_steps + self.getup_steps)
            + 2 * u64::from(self.clinch_steps + self.getup_steps)
    }

    fn validate(&self) -> Result<(), TapeError> {
        if self.falls == 0 {
            return Err(TapeError::NoFalls);
        }
        if self.getup_steps == 0
            || self.clinch_steps == 0
            || self.fall_steps < HIT_MIN_TICK + HIT_TAIL_TICKS + 1
        {
            return Err(TapeError::NoSteps);
        }
        Ok(())
    }
}

/// Baseline numbers for one tape run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TapeReport {
    /// Falls run.
    pub falls: usize,
    /// Master seed run under.
    pub seed: u64,
    /// `Rng::digest` over the per-fall end hashes plus the clinch hash, in
    /// run order. Identical across same-config runs (the hash-x2 gate).
    pub digest: u64,
    /// Deepest bone-center-into-floor penetration seen, meters; `0.0` when
    /// no bone center crossed the floor plane. Center-based probe: the
    /// capsule axis is ignored, so this is a floor, never a tunneling proof
    /// (axis-aware overlap is the swept-cast detector's job).
    pub interpenetration_max_m: f32,
    /// Closest approach of any bone center across the two clinch fighters,
    /// meters, over the whole clinch. Starts at [`CLINCH_GAP_M`]; the seeded
    /// hit must close it.
    pub separation_min_m: f32,
    /// Deepest capsule-against-capsule overlap between the two clinch
    /// fighters, meters, read after the parting pass of every tick (axis
    /// aware, unlike the center probes above).
    pub clinch_overlap_max_m: f32,
    /// Most heap calls in any single step (sampler-measured; `0` for the
    /// plain runner, which never counts).
    pub per_tick_alloc_calls: u64,
    /// Most heap bytes in any single step, same sampling note as calls.
    pub per_tick_alloc_bytes: u64,
    /// Total `step` calls performed; always equals
    /// [`TapeConfig::ticks`].
    pub ticks: u64,
}

impl Display for TapeReport {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let Self {
            falls,
            seed,
            digest,
            interpenetration_max_m: inter,
            separation_min_m: sep,
            clinch_overlap_max_m: overlap,
            per_tick_alloc_calls: calls,
            per_tick_alloc_bytes: bytes,
            ticks,
        } = *self;
        write!(
            f,
            "TAPE falls={falls} seed={seed} digest={digest:016x} \
             inter_max={inter:.4}m sep_min={sep:.4}m overlap_max={overlap:.4}m \
             alloc_tick={calls}calls/{bytes}B ticks={ticks}"
        )
    }
}

/// Plain stepper: forwards the fixed step and reports zero heap cost.
/// The report's per-tick alloc fields read `0` under this stepper; pass a
/// counting [`StepSampler`] to [`run_tape_with`] to fill them.
struct Plain;

impl StepSampler for Plain {
    fn step(&mut self, ragdoll: &mut Ragdoll, dt: f32) -> Result<StepCost, TapeError> {
        ragdoll.step(dt)?;
        Ok(StepCost::ZERO)
    }
}

/// Run the tape with the plain stepper (no heap counting; the report's
/// per-tick alloc fields read `0`). Errors: forwards `TapeError` from
/// config validation or any ragdoll call.
pub fn run_tape(config: &TapeConfig) -> Result<TapeReport, TapeError> {
    run_tape_with(config, &mut Plain)
}

/// Run the tape through `sampler`, folding its per-step heap costs into the
/// report maxima. Same fixture and order as [`run_tape`], so the digest is
/// identical for the same config (the measurement never perturbs the sim).
/// Errors: forwards `TapeError` from config validation or any ragdoll call.
pub fn run_tape_with<S: StepSampler>(
    config: &TapeConfig,
    sampler: &mut S,
) -> Result<TapeReport, TapeError> {
    config.validate()?;
    let mut hashes = Vec::with_capacity(config.falls + 1);
    let mut inter_max = 0.0f32;
    let mut max_calls = 0u64;
    let mut max_bytes = 0u64;
    let mut ticks = 0u64;
    for fall in 0..config.falls {
        let sample = run_fall(config, fall, sampler)?;
        hashes.push(sample.hash);
        inter_max = inter_max.max(sample.penetration_max);
        max_calls = max_calls.max(sample.max_calls);
        max_bytes = max_bytes.max(sample.max_bytes);
        ticks += sample.ticks;
    }
    let clinch = run_clinch(config, sampler, ClinchMode::LIVE)?;
    hashes.push(clinch.hash);
    inter_max = inter_max.max(clinch.penetration_max);
    max_calls = max_calls.max(clinch.max_calls);
    max_bytes = max_bytes.max(clinch.max_bytes);
    ticks += clinch.ticks;
    Ok(TapeReport {
        falls: config.falls,
        seed: config.seed,
        digest: Rng::digest(&hashes),
        interpenetration_max_m: inter_max,
        separation_min_m: clinch.separation_min,
        clinch_overlap_max_m: clinch.overlap_max,
        per_tick_alloc_calls: max_calls,
        per_tick_alloc_bytes: max_bytes,
        ticks,
    })
}

/// Outcome of one script leg: end hash plus folded maxima.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LegSample {
    hash: u64,
    penetration_max: f32,
    separation_min: f32,
    overlap_max: f32,
    max_calls: u64,
    max_bytes: u64,
    ticks: u64,
}

/// Four-bone vertical chain fixture: root at `root`, each child +0.5 in Y.
fn chain_skeleton() -> Skeleton {
    Skeleton {
        joints: vec![
            Joint {
                name: "root".into(),
                parent: None,
            },
            Joint {
                name: "mid".into(),
                parent: Some(0),
            },
            Joint {
                name: "tip".into(),
                parent: Some(1),
            },
            Joint {
                name: "end".into(),
                parent: Some(2),
            },
        ],
    }
}

/// Chain pose with the root at `root` (children carry +0.5 Y offsets).
fn chain_pose(root: [f32; 3]) -> LocalPose {
    LocalPose {
        positions: vec![root, [0.0, 0.5, 0.0], [0.0, 0.5, 0.0], [0.0, 0.5, 0.0]],
        rotations: vec![[0.0, 0.0, 0.0, 1.0]; 4],
    }
}

fn chain_shapes() -> Vec<BoneShape> {
    vec![
        BoneShape {
            radius: BONE_RADIUS,
            half_height: BONE_HALF_HEIGHT,
        };
        4
    ]
}

fn spawn_on_floor(root: [f32; 3]) -> Result<Ragdoll, TapeError> {
    let skeleton = chain_skeleton();
    let mut ragdoll = Ragdoll::spawn(&skeleton, &chain_pose(root), &chain_shapes())?;
    ragdoll.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])?;
    ragdoll.blend_to_ragdoll(1.0)?;
    Ok(ragdoll)
}

/// Deterministic unit float in `[0, 1)` from a stream word: top 24 bits over
/// `2^24` (the crate's fixture idiom; precision loss is intended and the
/// mapping is identical on every platform).
fn unit(word: u64) -> f32 {
    // Top 24 bits over `2^24`; the split keeps the conversion exact on
    // every platform (`hi` holds 8 bits, `lo` 16 bits, both widen exactly
    // to `f32`, and their sum stays below `2^24` so it is exact).
    let top = word >> 40;
    let hi = u16::try_from(top >> 16).unwrap_or(u16::MAX);
    let lo = u16::try_from(top & 0xFFFF).unwrap_or(u16::MAX);
    (f32::from(hi) * 65_536.0 + f32::from(lo)) / 16_777_216.0
}

/// Deepest floor penetration over one ragdoll's bones, meters; `0.0` when
/// every bone center stays above the floor plane plus its radius.
fn penetration(ragdoll: &Ragdoll) -> f32 {
    let mut worst = 0.0f32;
    for i in 0..ragdoll.bone_count() {
        if let Some(p) = ragdoll.bone_position(i) {
            let depth = BONE_RADIUS - (p[1] - FLOOR_TOP_Y);
            worst = worst.max(depth);
        }
    }
    worst
}

/// Closest bone-center approach across two fighters, meters.
fn separation(a: &Ragdoll, b: &Ragdoll) -> f32 {
    let mut best = f32::INFINITY;
    for i in 0..a.bone_count() {
        let Some(pa) = a.bone_position(i) else {
            continue;
        };
        for j in 0..b.bone_count() {
            let Some(pb) = b.bone_position(j) else {
                continue;
            };
            let dx = pa[0] - pb[0];
            let dy = pa[1] - pb[1];
            let dz = pa[2] - pb[2];
            // One `sqrt` per pair, allowed sim math.
            best = best.min((dx * dx + dy * dy + dz * dz).sqrt());
        }
    }
    best
}

/// Step once through the sampler, folding the heap cost maxima.
fn sampled<S: StepSampler>(
    ragdoll: &mut Ragdoll,
    sampler: &mut S,
    costs: &mut (u64, u64),
) -> Result<(), TapeError> {
    let cost = sampler.step(ragdoll, FIXED_DT)?;
    costs.0 = costs.0.max(cost.calls);
    costs.1 = costs.1.max(cost.bytes);
    Ok(())
}

/// Blend back to the animated pose over the get-up window while stepping.
fn get_up<S: StepSampler>(
    ragdoll: &mut Ragdoll,
    sampler: &mut S,
    getup_steps: u32,
    costs: &mut (u64, u64),
    penetration_max: &mut f32,
) -> Result<u64, TapeError> {
    let mut ticks = 0u64;
    for g in 0..getup_steps {
        // Tick counters ride the get-up window (baseline 60 ticks); every
        // reachable count fits in `u16`, so the saturating widening matches
        // the old cast exactly and an absurd count clamps instead of wrapping.
        let step = f32::from(u16::try_from(g).unwrap_or(u16::MAX));
        let window = f32::from(u16::try_from(getup_steps).unwrap_or(u16::MAX));
        let weight = 1.0 - (step + 1.0) / window;
        ragdoll.blend_to_ragdoll(weight)?;
        sampled(ragdoll, sampler, costs)?;
        *penetration_max = (*penetration_max).max(penetration(ragdoll));
        ticks += 1;
    }
    Ok(ticks)
}

/// One seeded fall: lateral spawn offset, a seeded mid-fall hit, then the
/// get-up blend-back. Stream `fall` keeps falls independent of order.
fn run_fall<S: StepSampler>(
    config: &TapeConfig,
    fall: usize,
    sampler: &mut S,
) -> Result<LegSample, TapeError> {
    // `fall` is a loop counter below `config.falls`; the saturating widening
    // matches the old cast exactly on every reachable config.
    let stream = u64::try_from(fall).unwrap_or(u64::MAX);
    let mut rng = Rng::new(config.seed).fork(stream);
    let root = [
        unit(rng.next_u64()) * 2.0 * SPAWN_HALF_RANGE - SPAWN_HALF_RANGE,
        ROOT_HEIGHT,
        unit(rng.next_u64()) * 2.0 * SPAWN_HALF_RANGE - SPAWN_HALF_RANGE,
    ];
    // Remainder below the `u32` window width, so the checked narrowing is
    // exact and never clamps.
    let hit_tick = HIT_MIN_TICK
        + u32::try_from(
            rng.next_u64() % u64::from(config.fall_steps - HIT_MIN_TICK - HIT_TAIL_TICKS),
        )
        .unwrap_or(u32::MAX);
    // Remainder below 4, so the checked conversion is exact.
    let hit_bone = usize::try_from(rng.next_u64() % 4).unwrap_or(usize::MAX);
    let impulse = [
        unit(rng.next_u64()) * 2.0 * HIT_HALF_RANGE - HIT_HALF_RANGE,
        unit(rng.next_u64()) * 2.0 * HIT_HALF_RANGE - HIT_HALF_RANGE,
        unit(rng.next_u64()) * 2.0 * HIT_HALF_RANGE - HIT_HALF_RANGE,
    ];
    let mut ragdoll = spawn_on_floor(root)?;
    let mut costs = (0u64, 0u64);
    let mut penetration_max = 0.0f32;
    let mut ticks = 0u64;
    for t in 0..config.fall_steps {
        if t == hit_tick {
            ragdoll.apply_hit(hit_bone, impulse, u64::from(t))?;
        }
        sampled(&mut ragdoll, sampler, &mut costs)?;
        penetration_max = penetration_max.max(penetration(&ragdoll));
        ticks += 1;
    }
    ticks += get_up(
        &mut ragdoll,
        sampler,
        config.getup_steps,
        &mut costs,
        &mut penetration_max,
    )?;
    Ok(LegSample {
        hash: ragdoll.state_hash(),
        penetration_max,
        separation_min: f32::INFINITY,
        overlap_max: 0.0,
        max_calls: costs.0,
        max_bytes: costs.1,
        ticks,
    })
}

/// How the clinch leg runs. The tape always runs [`ClinchMode::LIVE`]; the tests flip the
/// switches to show what the parting pass and its fighter order change.
#[derive(Clone, Copy)]
struct ClinchMode {
    /// Run the parting pass after every tick; off, the two fighters pass through each other.
    part: bool,
    /// Hand the fighters to the parting pass as B then A instead of A then B.
    shuffled: bool,
}

impl ClinchMode {
    /// What the tape runs: fighters parted, handed over A then B.
    const LIVE: Self = Self {
        part: true,
        shuffled: false,
    };
}

/// The clinch: two fighters spawn [`CLINCH_GAP_M`] apart, fall together,
/// fighter B takes a seeded shove toward A, then both get up. The fighters
/// live in separate worlds, so the parting pass ([`part_members`]) is the only contact
/// between them; the separation posts the closest approach of the two clouds and the overlap
/// the deepest capsule sink after that pass.
fn run_clinch<S: StepSampler>(
    config: &TapeConfig,
    sampler: &mut S,
    mode: ClinchMode,
) -> Result<LegSample, TapeError> {
    let mut rng = Rng::new(config.seed).fork_named("clinch");
    // Remainder below 90, so the checked narrowing is exact.
    let hit_tick = HIT_MIN_TICK + u32::try_from(rng.next_u64() % 90).unwrap_or(u32::MAX);
    let shove = 0.03 + unit(rng.next_u64()) * 0.05;
    let mut a = spawn_on_floor([-CLINCH_GAP_M * 0.5, ROOT_HEIGHT, 0.0])?;
    let mut b = spawn_on_floor([CLINCH_GAP_M * 0.5, ROOT_HEIGHT, 0.0])?;
    let mut costs = (0u64, 0u64);
    let mut penetration_max = 0.0f32;
    let mut separation_min = f32::INFINITY;
    let mut overlap_max = 0.0f32;
    let mut ticks = 0u64;
    for t in 0..config.clinch_steps {
        if t == hit_tick {
            b.apply_hit(1, [-shove, 0.0, 0.0], u64::from(t))?;
        }
        // Fixed A-then-B order: step order never reaches a hash.
        sampled(&mut a, sampler, &mut costs)?;
        sampled(&mut b, sampler, &mut costs)?;
        if mode.part {
            let mut pair = [
                Member {
                    id: 0,
                    ragdoll: &mut a,
                },
                Member {
                    id: 1,
                    ragdoll: &mut b,
                },
            ];
            if mode.shuffled {
                pair.swap(0, 1);
            }
            part_members(&mut pair);
        }
        penetration_max = penetration_max.max(penetration(&a));
        penetration_max = penetration_max.max(penetration(&b));
        separation_min = separation_min.min(separation(&a, &b));
        overlap_max = overlap_max.max(deepest_overlap(&a, &b));
        ticks += 2;
    }
    ticks += get_up(
        &mut a,
        sampler,
        config.getup_steps,
        &mut costs,
        &mut penetration_max,
    )?;
    ticks += get_up(
        &mut b,
        sampler,
        config.getup_steps,
        &mut costs,
        &mut penetration_max,
    )?;
    Ok(LegSample {
        hash: Rng::digest(&[a.state_hash(), b.state_hash()]),
        penetration_max,
        separation_min,
        overlap_max,
        max_calls: costs.0,
        max_bytes: costs.1,
        ticks,
    })
}

#[cfg(test)]
mod tests {
    use super::super::clinch::ClinchStats;
    use super::super::powered::GETUP_BUDGET_TICKS;
    use super::*;
    use engine_core::alloc_probe;

    /// Counting sampler: measures every fixed step's heap use and folds the
    /// maxima, mirroring the crate's `alloc_probe` idiom.
    #[derive(Default)]
    struct Probe {
        max_calls: u64,
        max_bytes: u64,
    }

    impl StepSampler for Probe {
        fn step(&mut self, ragdoll: &mut Ragdoll, dt: f32) -> Result<StepCost, TapeError> {
            let (result, count) = alloc_probe::measure(|| ragdoll.step(dt));
            result?;
            self.max_calls = self.max_calls.max(count.calls);
            self.max_bytes = self.max_bytes.max(count.bytes);
            Ok(StepCost {
                calls: count.calls,
                bytes: count.bytes,
            })
        }
    }

    #[test]
    fn getup_window_matches_powered_budget() {
        assert_eq!(u64::from(GETUP_STEPS), GETUP_BUDGET_TICKS);
    }

    #[test]
    fn config_ticks_counts_every_step() {
        let config = TapeConfig::baseline();
        assert_eq!(
            config.ticks(),
            u64::try_from(FALLS).unwrap_or(u64::MAX) * u64::from(FALL_STEPS + GETUP_STEPS)
                + 2 * u64::from(CLINCH_STEPS + GETUP_STEPS)
        );
    }

    #[test]
    fn bad_configs_fail_without_running() {
        assert_eq!(
            TapeConfig::new(TAPE_SEED, 0, FALL_STEPS, GETUP_STEPS, CLINCH_STEPS),
            Err(TapeError::NoFalls)
        );
        assert_eq!(
            TapeConfig::new(TAPE_SEED, 2, 10, GETUP_STEPS, CLINCH_STEPS),
            Err(TapeError::NoSteps)
        );
        assert_eq!(
            TapeConfig::new(TAPE_SEED, 2, FALL_STEPS, 0, CLINCH_STEPS),
            Err(TapeError::NoSteps)
        );
        assert_eq!(
            TapeConfig::new(TAPE_SEED, 2, FALL_STEPS, GETUP_STEPS, 0),
            Err(TapeError::NoSteps)
        );
    }

    /// Acceptance tape: 50 seeded falls plus the clinch tape at `FIXED_DT`,
    /// seed pinned. Posts interpenetration max, two-ragdoll separation and
    /// per-tick allocs; the digest must match across two runs (hash x2) and
    /// across steppers (the measurement never perturbs the sim).
    #[test]
    fn ragdoll_gap() {
        let config = TapeConfig::baseline();
        let mut first = Probe::default();
        let a = run_tape_with(&config, &mut first).expect("tape runs");
        let mut second = Probe::default();
        let b = run_tape_with(&config, &mut second).expect("tape runs again");
        // Hash x2: same seed, same order, same digest and same full report.
        assert_eq!(a.digest, b.digest, "tape digest must repeat: {a} vs {b}");
        assert_eq!(a, b, "tape report must repeat exactly");
        // Allocation behavior is deterministic too.
        assert_eq!(first.max_calls, second.max_calls);
        assert_eq!(first.max_bytes, second.max_bytes);
        // The plain stepper (no counting) must hash identically: the probe
        // is read-only on the tick.
        let plain = run_tape(&config).expect("plain tape runs");
        assert_eq!(plain.digest, a.digest, "counting must not perturb the sim");
        assert_eq!(plain.ticks, config.ticks());
        assert_eq!(a.ticks, config.ticks());
        // Floor oracle: bones rest on the floor, never through it.
        assert!(
            (0.0..PENETRATION_CAP_M).contains(&a.interpenetration_max_m),
            "penetration out of bounds: {}",
            a.interpenetration_max_m
        );
        // Clinch oracle: the shove closes the spawn gap but never inverts it.
        assert!(
            a.separation_min_m >= 0.0 && a.separation_min_m <= CLINCH_GAP_M,
            "separation out of bounds: {}",
            a.separation_min_m
        );
        assert!(
            a.separation_min_m < CLINCH_GAP_M - 0.05,
            "the clinch shove must close the gap: {}",
            a.separation_min_m
        );
        println!("{a}");
    }

    /// NS3 acceptance: once parted, no clinch bone sinks 1 cm into a bone of the other
    /// fighter; the clinch repeats bit for bit and hands the same result whichever fighter the
    /// pass sees first; and the control run without the pass does sink, so the bound has teeth.
    #[test]
    fn clinch_fighters_part_under_a_centimeter_x2_shuffled() {
        let config = TapeConfig::baseline();
        let run = |part, shuffled| {
            run_clinch(&config, &mut Plain, ClinchMode { part, shuffled }).expect("clinch runs")
        };
        let live = run(true, false);
        assert_eq!(live, run(true, false), "the clinch must repeat");
        assert_eq!(
            live,
            run(true, true),
            "fighter order must not reach the result"
        );
        assert!(
            live.overlap_max < 0.01,
            "fighters sank {} m into each other",
            live.overlap_max
        );
        let free = run(false, false);
        assert!(
            free.overlap_max > 0.01,
            "the control must interpenetrate: {} m",
            free.overlap_max
        );
        println!(
            "clinch overlap max {:.4} m parted, {:.4} m unparted",
            live.overlap_max, free.overlap_max
        );
    }

    #[test]
    fn parting_pushes_two_fighters_that_start_inside_each_other_apart() {
        // Two chains 0.2 m apart with 0.15 m bones sink 0.1 m into each other everywhere.
        let mut a = spawn_on_floor([-0.1, ROOT_HEIGHT, 0.0]).expect("a");
        let mut b = spawn_on_floor([0.1, ROOT_HEIGHT, 0.0]).expect("b");
        let before = deepest_overlap(&a, &b);
        assert!(before > 0.09, "the fixture must overlap: {before}");
        let mut pair = [
            Member {
                id: 0,
                ragdoll: &mut a,
            },
            Member {
                id: 1,
                ragdoll: &mut b,
            },
        ];
        let stats = part_members(&mut pair);
        assert!(stats.parted > 0 && stats.deepest_m > 0.09, "{stats:?}");
        let after = deepest_overlap(&a, &b);
        assert!(after < 0.01, "still {after} m deep after parting");
    }

    #[test]
    fn parting_leaves_fighters_that_do_not_touch_alone() {
        let mut a = spawn_on_floor([-2.0, ROOT_HEIGHT, 0.0]).expect("a");
        let mut b = spawn_on_floor([2.0, ROOT_HEIGHT, 0.0]).expect("b");
        let before = (a.state_hash(), b.state_hash());
        let mut pair = [
            Member {
                id: 1,
                ragdoll: &mut b,
            },
            Member {
                id: 0,
                ragdoll: &mut a,
            },
        ];
        assert_eq!(part_members(&mut pair), ClinchStats::default());
        assert_eq!(pair[0].id, 0, "the pass sorts the fighters by id");
        assert_eq!((a.state_hash(), b.state_hash()), before);
        assert_eq!(deepest_overlap(&a, &b), 0.0);
    }
}
