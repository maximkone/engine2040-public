//! tick: fixed 60Hz driver. Construct -> step -> hash; fixed dt in, never wall-clock.
//! Owner: engine-core. Default accumulator is integer [`crate::time::IntAcc`];
//! f32 `LegacyFixedAcc` (`LEGACY_DT`) is frozen for receipts only.
use crate::{hash_bytes, Rng};
/// Ticks per second.
pub const TICK_HZ: u64 = 60;
/// Frozen legacy step in seconds (1.0 / 60.0). Bits never change.
pub const LEGACY_DT: f32 = 1.0 / 60.0;
/// Fixed step in seconds (legacy alias, identical bits to `LEGACY_DT`).
/// New code uses integer [`crate::time::IntAcc`]; this stays for receipts.
pub const DT: f32 = LEGACY_DT;
/// Max fixed steps per frame (anti-spiral cap).
pub const MAX_STEPS: u32 = 5;
/// Frozen legacy frame clamp in seconds. Bits never change.
pub const LEGACY_MAX_FRAME_DT: f32 = 0.1;
/// Max accepted frame dt in seconds (legacy alias, identical bits to legacy).
pub const MAX_FRAME_DT: f32 = LEGACY_MAX_FRAME_DT;
/// Default tick accumulator: integer ns path, zero drift.
/// Legacy f32 path stays under `LegacyFixedAcc` for receipts only.
pub type DefaultAcc = crate::time::IntAcc;
/// Tick index since sim start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tick(pub u64);
/// Frozen legacy f32 accumulator: acc += min(frame_dt, 0.1), step while acc >= dt capped at 5.
/// Returns steps taken. Render alpha is acc / dt. Cap hit drops backlog (no spiral).
/// Frozen: behavior and bits never change. New code uses integer [`crate::time::IntAcc`].
#[derive(Clone, Copy, Debug)]
pub struct LegacyFixedAcc {
    /// Buffered time in seconds.
    pub acc: f32,
    /// Fixed step in seconds.
    pub dt: f32,
}
impl LegacyFixedAcc {
    /// Build with an explicit step.
    pub fn new(dt: f32) -> Self {
        Self { acc: 0.0, dt }
    }
    /// Build with the 60Hz default step.
    pub fn fixed() -> Self {
        Self::new(LEGACY_DT)
    }
    /// Push frame time (never wall-clock inside), return steps due.
    pub fn push(&mut self, frame_dt: f32) -> u32 {
        self.acc += frame_dt.min(LEGACY_MAX_FRAME_DT);
        let mut n = 0;
        while self.acc >= self.dt && n < MAX_STEPS {
            self.acc -= self.dt;
            n += 1;
        }
        n
    }
    /// Blend factor for render interpolation in [0, 1].
    pub fn alpha(&self) -> f32 {
        (self.acc / self.dt).clamp(0.0, 1.0)
    }
}
/// Legacy float accumulator; new code uses [`crate::IntAcc`] for exact steps.
#[deprecated(note = "Use `crate::IntAcc` for fixed-step accumulation.")]
pub type FixedAcc = LegacyFixedAcc;
/// Deterministic tick state: construct -> step -> hash.
/// Warm steady-state path: `step` borrows nothing and allocates nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickState {
    /// Next tick index.
    pub tick: u64,
    /// Running hash (starts at seed).
    pub acc: u64,
    /// Seed folded into every hash.
    pub seed: u64,
}
impl TickState {
    /// Construct from a seed; hash starts at the seed.
    pub fn new(seed: u64) -> Self {
        Self {
            tick: 0,
            acc: seed,
            seed,
        }
    }
    /// Warm path: advance one tick, folding step_hash; returns the running hash.
    /// Allocates nothing; drive this directly per tick in production.
    pub fn step(&mut self, step_hash: u64) -> u64 {
        self.acc = fold_hash(self.acc, self.tick, step_hash);
        self.tick += 1;
        self.acc
    }
    /// Current running hash.
    pub fn hash(&self) -> u64 {
        self.acc
    }
}
/// Pure fold: mix one per-tick step_hash plus tick index into acc.
pub fn fold_hash(acc: u64, tick: u64, step_hash: u64) -> u64 {
    let mut h = acc;
    h ^= step_hash
        .wrapping_add(0x9e3779b97f4a7c15)
        .wrapping_mul(0xbf58476d1ce4e5b9);
    h ^= hash_bytes(&tick.to_le_bytes());
    h
}
/// Cold/test-only fixed-step driver: ticks steps of step, folding a running hash each tick.
/// Allocates exactly one `Vec` (the returned hashes); steady state is `TickState::step`,
/// which allocates nothing. Do not call per frame in production; drive `TickState` directly.
pub fn run_ticks<F>(ticks: u64, seed: u64, mut step: F) -> Vec<u64>
where
    F: FnMut(Tick, &mut Rng) -> u64,
{
    let mut rng = Rng::new(seed);
    let mut state = TickState::new(seed);
    let mut hashes = Vec::with_capacity(usize::try_from(ticks).expect("tick count fits in usize"));
    for t in 0..ticks {
        let h = step(Tick(t), &mut rng);
        hashes.push(state.step(h));
    }
    hashes
}
/// Failure of [`verify_same_seed`]: empty run or two runs that diverged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyError {
    /// No ticks were requested, so there is no final hash.
    Empty,
    /// The two same-seed runs diverged; carries the seed for diagnosis.
    Desync {
        /// Seed of the divergent runs.
        seed: u64,
    },
}

impl core::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Empty => write!(f, "verify: no ticks"),
            Self::Desync { seed } => write!(f, "desync seed={seed}"),
        }
    }
}

impl std::error::Error for VerifyError {}

/// Verify determinism: run make_hash twice on same seed, compare hashes.
/// Returns the final hash on match; [`VerifyError::Empty`] when `ticks == 0`,
/// [`VerifyError::Desync`] (carrying `seed`) when the runs diverge.
pub fn verify_same_seed<F>(ticks: u64, seed: u64, mut make_hash: F) -> Result<u64, VerifyError>
where
    F: FnMut(Tick, &mut Rng) -> u64,
{
    if ticks == 0 {
        return Err(VerifyError::Empty);
    }
    let h1 = run_ticks(ticks, seed, &mut make_hash);
    let h2 = run_ticks(ticks, seed, &mut make_hash);
    if h1 != h2 {
        return Err(VerifyError::Desync { seed });
    }
    h1.last().copied().ok_or(VerifyError::Empty)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::alloc_probe::measure;
    #[test]
    fn fixed_60hz_consts() {
        assert_eq!(TICK_HZ, 60);
        assert_eq!(MAX_STEPS, 5);
        assert!((DT - 1.0 / 60.0).abs() < f32::EPSILON);
        assert!((MAX_FRAME_DT - 0.1).abs() < f32::EPSILON);
    }
    #[test]
    fn acc_cap5_anti_spiral() {
        let mut a = LegacyFixedAcc::fixed();
        assert_eq!(a.push(10.0), 5);
        assert!(a.alpha() >= 0.0 && a.alpha() <= 1.0);
        let mut b = LegacyFixedAcc::fixed();
        assert_eq!(b.push(0.0), 0);
        assert_eq!(b.alpha(), 0.0);
    }
    #[test]
    fn acc_exact_steps() {
        let mut a = LegacyFixedAcc::fixed();
        assert_eq!(a.push(DT), 1);
        assert_eq!(a.push(DT * 2.5), 2);
    }
    #[test]
    fn run_600_seed2040_deterministic_x2() {
        let h1 = run_ticks(600, 2040, |t, _| t.0);
        let h2 = run_ticks(600, 2040, |t, _| t.0);
        assert_eq!(h1.len(), 600);
        assert_eq!(h1, h2);
        assert_ne!(h1.first(), h1.last());
    }
    #[test]
    fn tickstate_fold_matches_run() {
        let v = run_ticks(8, 2040, |t, _| t.0);
        let mut s = TickState::new(2040);
        for t in 0..8u64 {
            s.step(t);
        }
        assert_eq!(s.hash(), v[v.len() - 1]);
        assert_eq!(s.tick, 8);
    }
    #[test]
    fn seed_in_hash_out() {
        let a = run_ticks(16, 2040, |t, _| t.0);
        let b = run_ticks(16, 7, |t, _| t.0);
        assert_ne!(a, b);
    }
    #[test]
    fn verify_empty_is_err() {
        assert_eq!(
            verify_same_seed(0, 2040, |t, _| t.0),
            Err(VerifyError::Empty)
        );
    }
    #[test]
    fn verify_pinned_returns_final_hash() {
        let expected = *run_ticks(8, 2040, |t, _| t.0).last().unwrap();
        assert_eq!(verify_same_seed(8, 2040, |t, _| t.0), Ok(expected));
    }
    #[test]
    fn verify_desync_carries_seed() {
        let mut calls = 0u64;
        let res = verify_same_seed(4, 2040, |t, _| {
            calls += 1;
            if calls > 4 {
                t.0.wrapping_add(1000)
            } else {
                t.0
            }
        });
        assert_eq!(res, Err(VerifyError::Desync { seed: 2040 }));
    }
    #[test]
    fn run_ticks_allocates_one_vec() {
        let (hashes, probe) = measure(|| run_ticks(16, 2040, |t, _| t.0));
        assert_eq!(hashes.len(), 16);
        assert_eq!(probe.calls, 1, "run_ticks is one Vec per run");
    }
    #[test]
    fn tickstate_step_steady_state_allocates_nothing() {
        let mut state = TickState::new(2040);
        let mut rng = Rng::new(2040);
        // Warm once outside the probe so setup heap use is not counted.
        let _ = state.step(rng.next_u64());
        let ((), steady) = measure(|| {
            for t in 1..64u64 {
                let h = t ^ rng.next_u64();
                std::hint::black_box(state.step(h));
            }
            std::hint::black_box(state.hash());
        });
        assert_eq!(steady.calls, 0, "warm TickState::step must not allocate");
        assert_eq!(state.tick, 64);
    }
    #[test]
    #[expect(
        deprecated,
        reason = "alias-vs-legacy identity is the point of this test"
    )]
    fn legacy_bits_frozen_identical() {
        assert_eq!(LEGACY_DT.to_bits(), (1.0f32 / 60.0f32).to_bits());
        assert_eq!(DT.to_bits(), LEGACY_DT.to_bits());
        assert_eq!(LEGACY_MAX_FRAME_DT.to_bits(), 0.1f32.to_bits());
        assert_eq!(MAX_FRAME_DT.to_bits(), LEGACY_MAX_FRAME_DT.to_bits());
        let mut legacy = LegacyFixedAcc::fixed();
        let mut alias = FixedAcc::fixed();
        assert_eq!(legacy.acc.to_bits(), alias.acc.to_bits());
        assert_eq!(legacy.dt.to_bits(), alias.dt.to_bits());
        assert_eq!(legacy.push(DT), alias.push(LEGACY_DT));
        assert_eq!(legacy.push(DT * 2.5), alias.push(LEGACY_DT * 2.5));
        assert_eq!(legacy.alpha().to_bits(), alias.alpha().to_bits());
    }
    #[test]
    fn default_acc_is_integer_path() {
        let mut acc = DefaultAcc::new();
        let mut steps: u64 = 0;
        for _ in 0..1000 {
            steps += u64::from(acc.push_ns(1_000_000).expect("1ms bounded"));
        }
        assert_eq!(steps, 60);
        assert_eq!(acc.tick(), 60);
    }
}
