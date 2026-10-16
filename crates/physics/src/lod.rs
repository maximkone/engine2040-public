//! lod: physics LOD for armies. Far soldiers step as one capsule, near ones
//! step capsule plus bones, CCD fires only for fast weapons and projectiles,
//! and resting capsule islands sleep through the shared island sleeper.
//!
//! Integer millimetre thresholds keep tier assignment exact; iteration stays
//! in slot order, so the same distances plus the same bodies give the same
//! tiers and the same hash on every PC and in replay. No allocation happens
//! per tick: every buffer is owned by [`LodWorld`] and reused.
//!
//! Consumer: the combined-tick workload (64 fighters plus 500 agents,
//! physics slice under 4 ms) and the perf workloads call
//! [`LodWorld::new_army`], [`LodWorld::update_tiers`] and [`LodWorld::step`].
//! The full ragdoll solve stays in the rapier path; this module decides who
//! pays for it. Std-only, deterministic math only (`+ - * /`, comparisons).
#![forbid(unsafe_code)]

use engine_core::{Fnv64, Rng};

use crate::islands::{self, SleepConfig};
use crate::rigid::{self, Body};

/// Soldiers in the army fixture (row accept: 500 AI soldiers).
pub const SOLDIERS: usize = 500;
/// Duelists in the army fixture (row accept: 64 duelists, always full detail).
pub const FIGHTERS: usize = 64;
/// Simulated bones per full-detail body in the std workload.
pub const FULL_BONES: usize = 8;
/// Distance marker for "unknown": always classifies to [`Tier::Capsule`].
pub const DIST_UNKNOWN_MM: u32 = u32::MAX;
/// Fixture seed stream names; forks keep soldier/fighter draws independent.
const SEED_SOLDIERS: u64 = 0x50_1D_1E_55;
const SEED_FIGHTERS: u64 = 0xF164_0E25;

/// Detail tier of one soldier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Near: capsule plus bones step every tick, never sleeps.
    Full,
    /// Far: capsule only, sleeps with its island when the island rests.
    Capsule,
}

/// Continuous-collision request for one mover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ccd {
    /// Swept move this tick.
    On,
    /// Plain integrate; exact and cheaper.
    Off,
}

/// What a mover is. Only fast weapons and projectiles earn CCD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentKind {
    Soldier,
    Duelist,
    Weapon,
    Projectile,
}

/// LOD policy. Distances are millimetres (integer, exact); speeds m/s.
#[derive(Clone, Copy, Debug)]
pub struct LodConfig {
    /// At or below: capsule promotes to full.
    pub near_mm: u32,
    /// At or above: full demotes to capsule. Must exceed `near_mm`; the band
    /// between the two is hysteresis so borderline soldiers stop flapping.
    pub far_mm: u32,
    /// Speed at or above which a weapon or projectile earns CCD.
    pub ccd_speed_mps: f32,
    /// Shared island sleep hysteresis for capsule-tier soldiers.
    pub sleep: SleepConfig,
}

impl Default for LodConfig {
    fn default() -> Self {
        Self {
            near_mm: 15_000,
            far_mm: 20_000,
            ccd_speed_mps: 20.0,
            sleep: SleepConfig::default(),
        }
    }
}

/// LOD input errors. Typed so callers handle them without panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LodError {
    /// Distance slice length differs from [`SOLDIERS`].
    LenMismatch { want: usize, got: usize },
    /// Thresholds inverted (`far_mm <= near_mm`): hysteresis needs a band.
    InvertedBand { near_mm: u32, far_mm: u32 },
    /// CCD speed threshold is NaN or negative: no mover could earn CCD.
    BadCcdSpeed,
    /// Soldier slot outside the army (`index >= SOLDIERS`).
    BadIndex { index: usize, len: usize },
    /// Step `dt` is zero, negative or NaN: the tick must pass fixed dt.
    BadDt,
}

impl core::fmt::Display for LodError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::LenMismatch { want, got } => {
                write!(f, "lod distances: want {want}, got {got}")
            }
            Self::InvertedBand { near_mm, far_mm } => {
                write!(f, "lod band inverted: near {near_mm}mm >= far {far_mm}mm")
            }
            Self::BadCcdSpeed => write!(f, "lod ccd speed invalid: want finite >= 0 m/s"),
            Self::BadIndex { index, len } => {
                write!(f, "lod soldier index out of range: {index} >= {len}")
            }
            Self::BadDt => write!(f, "lod step dt invalid: want finite > 0"),
        }
    }
}

/// Check one config: the hysteresis band must be non-empty.
pub fn check_config(cfg: LodConfig) -> Result<(), LodError> {
    if cfg.far_mm <= cfg.near_mm {
        return Err(LodError::InvertedBand {
            near_mm: cfg.near_mm,
            far_mm: cfg.far_mm,
        });
    }
    if !cfg.ccd_speed_mps.is_finite() || cfg.ccd_speed_mps < 0.0 {
        return Err(LodError::BadCcdSpeed);
    }
    Ok(())
}

/// Classify one soldier with hysteresis. Unknown distance fails safe to
/// [`Tier::Capsule`] (cheap, never a ragdoll bill for a lost tracker).
pub fn classify(current: Tier, dist_mm: u32, cfg: &LodConfig) -> Tier {
    if dist_mm <= cfg.near_mm {
        Tier::Full
    } else if dist_mm >= cfg.far_mm {
        Tier::Capsule
    } else {
        current
    }
}

/// CCD policy: fast weapons and projectiles only. Fast bodies of any other
/// kind stay [`Ccd::Off`]; slow weapons never tunnel far enough to matter.
pub fn ccd_for(kind: AgentKind, speed_mps: f32, cfg: &LodConfig) -> Ccd {
    let armed = matches!(kind, AgentKind::Weapon | AgentKind::Projectile);
    if armed && speed_mps.is_finite() && speed_mps >= cfg.ccd_speed_mps {
        Ccd::On
    } else {
        Ccd::Off
    }
}

/// Sleep-skip for far, still, grounded capsules beside the island sleeper.
///
/// A [`Tier::Capsule`] body that [`rigid::is_settled`] holds for integrates
/// to identical bytes, so skipping it never moves the hash: bit-exact
/// `y == hy`, zero velocities, finite `x` never `-0.0`, and `|hy| < 32768`
/// so gravity cannot re-activate below-ulp motion. Full-tier bodies never
/// skip; any body at or above the wake threshold never skips, so a kick
/// always wakes. An already-asleep body must also be settled: an unsettled
/// sleeper (`x` non-finite/`-0.0`, signed-zero `y`/`hy` mismatch, large
/// `hy`) normalizes on step, so staying asleep would fork bytes.
///
/// Consumer: [`LodWorld::step`] fast-paths far still bodies into sleep in one
/// tick instead of waiting `sleep_steps`; the 10k static cloud in the perf
/// harness is this shape once settled. Units: metres, m/s. Determinism: exact
/// bit compares plus `+ *` only, slot order, no allocation, no wall clock.
pub fn static_sleep_skip(body: &Body, tier: Tier, asleep: bool, cfg: &LodConfig) -> bool {
    if tier == Tier::Full {
        return false;
    }
    let v2 = body.vx * body.vx + body.vy * body.vy;
    if v2 >= cfg.sleep.wake_v2 {
        return false;
    }
    if asleep {
        return rigid::is_settled(body);
    }
    rigid::is_settled(body)
}

/// Army LOD world: 500 soldier capsules plus preallocated bone sets, 64
/// fighter bone sets, tier and sleep state per soldier. Buffers are fixed at
/// construction; [`LodWorld::step`] allocates nothing.
pub struct LodWorld {
    cfg: LodConfig,
    tiers: Vec<Tier>,
    capsules: Vec<Body>,
    soldier_bones: Vec<Body>,
    fighter_bones: Vec<Body>,
    timers: Vec<u32>,
    asleep: Vec<bool>,
    scratch_v2: Vec<f32>,
}

impl LodWorld {
    /// Deterministic army fixture from `seed`: the same seed builds the same
    /// bodies, so replay and re-verify start from identical bytes.
    pub fn new_army(seed: u64, cfg: LodConfig) -> Self {
        let mut soldier_rng = Rng::new(seed).fork(SEED_SOLDIERS);
        let mut fighter_rng = Rng::new(seed).fork(SEED_FIGHTERS);
        let mut capsules = Vec::with_capacity(SOLDIERS);
        let mut soldier_bones = Vec::with_capacity(SOLDIERS * FULL_BONES);
        for _ in 0..SOLDIERS {
            let cx = range_m(&mut soldier_rng, -40_000, 40_000);
            let cy = range_m(&mut soldier_rng, 500, 4_000);
            capsules.push(Body::new(cx, cy, 0.0, 0.0, 0.4, 0.9));
            // Bound: `b < FULL_BONES (8) <= u8::MAX`, so the fallible
            // conversion never fails; the default is dead code kept
            // so the checked conversion never affects bytes.
            for b in 0..FULL_BONES {
                let bone: u8 = u8::try_from(b).unwrap_or_default();
                let lift = f32::from(bone) * 0.22;
                soldier_bones.push(Body::new(cx, cy + lift, 0.0, 0.0, 0.15, 0.15));
            }
        }
        let mut fighter_bones = Vec::with_capacity(FIGHTERS * FULL_BONES);
        for _ in 0..FIGHTERS {
            let cx = range_m(&mut fighter_rng, -10_000, 10_000);
            // Bound: same as above, `b < FULL_BONES (8) <= u8::MAX`.
            for b in 0..FULL_BONES {
                let bone: u8 = u8::try_from(b).unwrap_or_default();
                let lift = f32::from(bone) * 0.22;
                fighter_bones.push(Body::new(cx, 1.0 + lift, 0.0, 0.0, 0.15, 0.15));
            }
        }
        Self {
            cfg,
            tiers: vec![Tier::Capsule; SOLDIERS],
            capsules,
            soldier_bones,
            fighter_bones,
            timers: vec![0; SOLDIERS],
            asleep: vec![false; SOLDIERS],
            scratch_v2: vec![0.0; SOLDIERS],
        }
    }

    /// Active policy.
    pub fn config(&self) -> LodConfig {
        self.cfg
    }

    /// Current tier of soldier `i`. Fails loud on a bad slot so a stale
    /// handle never reads a neighbour's tier.
    pub fn tier(&self, i: usize) -> Result<Tier, LodError> {
        self.tiers.get(i).copied().ok_or(LodError::BadIndex {
            index: i,
            len: SOLDIERS,
        })
    }

    /// Sleep state of soldier `i` (capsule tier only; full tier never sleeps).
    pub fn is_asleep(&self, i: usize) -> Result<bool, LodError> {
        self.asleep.get(i).copied().ok_or(LodError::BadIndex {
            index: i,
            len: SOLDIERS,
        })
    }

    /// How many soldiers ride full detail this tick.
    pub fn full_count(&self) -> usize {
        self.tiers.iter().filter(|&&t| t == Tier::Full).count()
    }

    /// Reassign every soldier tier from millimetre distances in slot order.
    /// Slot order iteration keeps the result identical on every machine.
    pub fn update_tiers(&mut self, dist_mm: &[u32]) -> Result<(), LodError> {
        if dist_mm.len() != SOLDIERS {
            return Err(LodError::LenMismatch {
                want: SOLDIERS,
                got: dist_mm.len(),
            });
        }
        check_config(self.cfg)?;
        for (i, &d) in dist_mm.iter().enumerate() {
            self.tiers[i] = classify(self.tiers[i], d, &self.cfg);
        }
        Ok(())
    }

    /// Step the world by `dt` (pass [`FIXED_DT`] on the tick):
    /// rebuild capsule islands from `pairs`, fold sleep, integrate awake
    /// capsules, bones of full-tier soldiers, and all fighter bones.
    /// `pairs` holds soldier-slot contact pairs; canonicalization inside
    /// [`islands::build_islands`] makes any input order hash-identical.
    /// A bad `dt` (zero, negative, NaN, infinite) is a loud [`LodError::BadDt`]:
    /// silently skipping the tick would fork replay.
    pub fn step(&mut self, dt: f32, pairs: &[(usize, usize)]) -> Result<(), LodError> {
        if dt <= 0.0 || !dt.is_finite() {
            return Err(LodError::BadDt);
        }
        let h = dt.min(0.25);
        let islands = islands::build_islands(SOLDIERS, pairs);
        for (i, b) in self.capsules.iter().enumerate() {
            self.scratch_v2[i] = b.vx * b.vx + b.vy * b.vy;
        }
        // Sleep-skip hunk beside the LOD tiers: far still grounded capsules
        // sleep in one tick instead of waiting `sleep_steps`. The skip is
        // idempotent (ground clamp restores the same bytes) so awake bodies
        // hash identically. Island wake runs after, so a kick wakes the whole
        // island even when mates are still.
        for (i, body) in self.capsules.iter().enumerate() {
            if self.tiers[i] != Tier::Full
                && !self.asleep[i]
                && static_sleep_skip(body, self.tiers[i], false, &self.cfg)
            {
                self.asleep[i] = true;
                self.timers[i] = self.cfg.sleep.sleep_steps;
            }
        }
        islands::update_sleep(
            SOLDIERS,
            &islands,
            &self.scratch_v2,
            &mut self.timers,
            &mut self.asleep,
            self.cfg.sleep,
        );
        for (i, body) in self.capsules.iter_mut().enumerate() {
            if self.tiers[i] == Tier::Full || !self.asleep[i] {
                rigid::step_body(body, h);
            }
        }
        for (i, chunk) in self.soldier_bones.chunks_exact_mut(FULL_BONES).enumerate() {
            if self.tiers[i] == Tier::Full {
                rigid::integrate(chunk, h);
            }
        }
        rigid::integrate(&mut self.fighter_bones, h);
        Ok(())
    }

    /// Deterministic state hash: tiers in slot order, then every body.
    /// Unstepped far bones still fold in, so the bytes stay complete.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        for &t in &self.tiers {
            h.write_u8(u8::from(t == Tier::Full));
        }
        h.write_u64(rigid::state_hash(&self.capsules));
        h.write_u64(rigid::state_hash(&self.soldier_bones));
        h.write_u64(rigid::state_hash(&self.fighter_bones));
        h.finish()
    }
}

/// Deterministic fixture coordinate in metres from integer millimetres.
/// Exact: the offset stays an integer below 2^16, so the `u16` trip keeps
/// every bit and the result matches the old float cast on all fixtures.
fn range_m(rng: &mut Rng, lo_mm: i64, hi_mm: i64) -> f32 {
    // Bounds: callers pass lo/hi inside [-40000, 40000], so `span >= 1`
    // fits `u64`, `rng.next_u64() % span < span` fits `i64`, and
    // `mm.abs() <= 40000 <= u16::MAX`. Each default below is dead
    // code kept only to keep the checked conversion total; bytes never change.
    let span = u64::try_from((hi_mm - lo_mm).max(1)).unwrap_or(1);
    let off = i64::try_from(rng.next_u64() % span).unwrap_or_default();
    let mm = lo_mm + off;
    let sign = if mm < 0 { -1.0 } else { 1.0 };
    let mag: u16 = u16::try_from(mm.abs()).unwrap_or_default();
    sign * f32::from(mag) / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rigid::FIXED_DT;

    fn tiers(dist_mm: u32) -> Vec<u32> {
        vec![dist_mm; SOLDIERS]
    }

    #[test]
    fn lod_hysteresis_promotes_holds_demotes() {
        let cfg = LodConfig::default();
        assert_eq!(classify(Tier::Capsule, cfg.near_mm, &cfg), Tier::Full);
        assert_eq!(classify(Tier::Full, cfg.far_mm, &cfg), Tier::Capsule);
        // Inside the band the current tier holds: no borderline flapping.
        let mid = cfg.near_mm + (cfg.far_mm - cfg.near_mm) / 2;
        assert_eq!(classify(Tier::Capsule, mid, &cfg), Tier::Capsule);
        assert_eq!(classify(Tier::Full, mid, &cfg), Tier::Full);
        // Unknown distance fails safe to the cheap tier.
        assert_eq!(classify(Tier::Full, DIST_UNKNOWN_MM, &cfg), Tier::Capsule);
    }

    #[test]
    fn lod_ccd_only_fast_weapons_and_projectiles() {
        let cfg = LodConfig::default();
        let fast = cfg.ccd_speed_mps + 5.0;
        let slow = (cfg.ccd_speed_mps - 1.0).max(0.0);
        assert_eq!(ccd_for(AgentKind::Weapon, fast, &cfg), Ccd::On);
        assert_eq!(ccd_for(AgentKind::Projectile, fast, &cfg), Ccd::On);
        assert_eq!(ccd_for(AgentKind::Weapon, slow, &cfg), Ccd::Off);
        assert_eq!(ccd_for(AgentKind::Soldier, fast, &cfg), Ccd::Off);
        assert_eq!(ccd_for(AgentKind::Duelist, fast, &cfg), Ccd::Off);
        assert_eq!(ccd_for(AgentKind::Projectile, f32::NAN, &cfg), Ccd::Off);
    }

    #[test]
    fn lod_len_mismatch_and_inverted_band_fail() {
        let mut w = LodWorld::new_army(7, LodConfig::default());
        let err = w
            .update_tiers(&[1_000; 8])
            .expect_err("short slice must fail");
        assert_eq!(
            err,
            LodError::LenMismatch {
                want: SOLDIERS,
                got: 8
            }
        );
        assert_eq!(
            format!("{err}"),
            format!("lod distances: want {SOLDIERS}, got 8")
        );
        let bad = LodConfig {
            near_mm: 20_000,
            far_mm: 20_000,
            ..LodConfig::default()
        };
        assert_eq!(
            check_config(bad),
            Err(LodError::InvertedBand {
                near_mm: 20_000,
                far_mm: 20_000
            })
        );
        let mut w2 = LodWorld::new_army(7, bad);
        w2.update_tiers(&tiers(1_000))
            .expect_err("inverted band must refuse tiers");
        // A bad CCD threshold gets its own error, never the band one.
        let bad_ccd = LodConfig {
            ccd_speed_mps: f32::NAN,
            ..LodConfig::default()
        };
        assert_eq!(check_config(bad_ccd), Err(LodError::BadCcdSpeed));
        let bad_neg = LodConfig {
            ccd_speed_mps: -1.0,
            ..LodConfig::default()
        };
        assert_eq!(check_config(bad_neg), Err(LodError::BadCcdSpeed));
    }

    #[test]
    fn lod_bad_index_and_bad_dt_fail() {
        let mut w = LodWorld::new_army(7, LodConfig::default());
        // Slot 0 starts cheap and awake; out-of-range slots fail loud.
        assert_eq!(w.tier(0), Ok(Tier::Capsule));
        assert_eq!(w.is_asleep(0), Ok(false));
        assert_eq!(
            w.tier(SOLDIERS),
            Err(LodError::BadIndex {
                index: SOLDIERS,
                len: SOLDIERS
            })
        );
        assert_eq!(
            w.is_asleep(SOLDIERS + 3),
            Err(LodError::BadIndex {
                index: SOLDIERS + 3,
                len: SOLDIERS
            })
        );
        // Zero, negative, NaN and infinite steps refuse instead of skipping.
        for &dt in &[0.0, -FIXED_DT, f32::NAN, f32::INFINITY] {
            assert_eq!(w.step(dt, &[]), Err(LodError::BadDt));
        }
        assert_eq!(
            format!("{}", LodError::BadDt),
            "lod step dt invalid: want finite > 0"
        );
    }

    #[test]
    fn lod_sleeping_capsules_skip_wake_on_kick() {
        let mut w = LodWorld::new_army(11, LodConfig::default());
        w.update_tiers(&tiers(DIST_UNKNOWN_MM)).expect("far");
        assert_eq!(w.full_count(), 0);
        let pair: Vec<(usize, usize)> = vec![(0, 1)];
        // Fall from up to 4 m (~54 ticks) plus the full sleep hysteresis.
        for _ in 0..300 {
            w.step(FIXED_DT, &pair).expect("step");
        }
        assert!(
            w.is_asleep(0).expect("slot 0") && w.is_asleep(1).expect("slot 1"),
            "rest must sleep the island"
        );
        // Sleeping capsules skip integration even at zero velocity: gravity
        // would move an integrated body, so unchanged `y` proves the skip.
        w.capsules[0].vy = 0.0;
        let y = w.capsules[0].y;
        w.asleep[0] = true;
        w.asleep[1] = true;
        w.step(FIXED_DT, &[]).expect("step");
        assert_eq!(w.capsules[0].y, y, "a sleeper must not integrate");
        // The velocity is the wake signal: the island wakes and moves.
        w.capsules[0].vy = 5.0;
        w.asleep[0] = true;
        w.asleep[1] = true;
        w.timers[0] = 0;
        w.timers[1] = 0;
        w.step(FIXED_DT, &pair).expect("step");
        assert!(
            !w.is_asleep(0).expect("slot 0") && !w.is_asleep(1).expect("slot 1"),
            "kick must wake the island"
        );
    }

    #[test]
    fn lod_static_10k_skip_wakes_on_kick() {
        // The 10k static cloud (perf render-10k/mixed, vx=vy=0) skips the
        // integrate path once settled: helper plus one-step island sleep.
        let cfg = LodConfig::default();
        let still = Body::new(1.0, 0.5, 0.0, 0.0, 0.5, 0.5);
        let mut skipped = 0_usize;
        for _ in 0..10_000 {
            if static_sleep_skip(&still, Tier::Capsule, false, &cfg) {
                skipped += 1;
            }
        }
        assert_eq!(skipped, 10_000, "10k still grounded must all skip");
        let mut kicked = still;
        kicked.vy = 5.0;
        assert!(
            !static_sleep_skip(&kicked, Tier::Capsule, true, &cfg),
            "kick must wake even from sleep"
        );
        assert!(
            !static_sleep_skip(&still, Tier::Full, false, &cfg),
            "full tier never skips"
        );
        // Airborne still must fall, never skip: skipping would fork the hash.
        let air = Body::new(1.0, 5.0, 0.0, 0.0, 0.5, 0.5);
        assert!(
            !static_sleep_skip(&air, Tier::Capsule, false, &cfg),
            "airborne still must integrate"
        );
        // Non-finite state never skips: the tick sanitizes it instead.
        let nan = Body::new(1.0, 0.5, f32::NAN, 0.0, 0.5, 0.5);
        assert!(
            !static_sleep_skip(&nan, Tier::Capsule, true, &cfg),
            "non-finite must integrate"
        );
        // Static-cloud shape (perf render-10k/mixed): far capsules, zero
        // velocity, grounded. The skip must sleep them in one step and wake
        // on kick without moving the hash for awake bodies.
        let mut w = LodWorld::new_army(77, LodConfig::default());
        w.update_tiers(&tiers(DIST_UNKNOWN_MM)).expect("far");
        for b in w.capsules.iter_mut() {
            b.x = 1.0;
            b.y = b.hy;
            b.vx = 0.0;
            b.vy = 0.0;
        }
        let before = crate::rigid::state_hash(&w.capsules);
        w.step(FIXED_DT, &[]).expect("step");
        assert!(
            w.is_asleep(0).expect("slot 0"),
            "far still grounded must sleep in one step"
        );
        assert_eq!(
            crate::rigid::state_hash(&w.capsules),
            before,
            "idempotent skip must not move capsule hash"
        );
        w.capsules[0].vy = 5.0;
        w.step(FIXED_DT, &[(0, 1)]).expect("step");
        assert!(
            !w.is_asleep(0).expect("slot 0") && !w.is_asleep(1).expect("slot 1"),
            "kick must wake the island"
        );
    }

    #[test]
    fn lod_static_skip_parity_on_edges() {
        // Skip must agree with `step_body` bytes on the four unsettled edges:
        // stepping normalizes each, so the skip must say integrate.
        let cfg = LodConfig::default();
        fn agrees(body: Body, asleep: bool, cfg: &LodConfig, label: &str) {
            let skip = static_sleep_skip(&body, Tier::Capsule, asleep, cfg);
            let mut stepped = body;
            crate::rigid::step_body(&mut stepped, FIXED_DT);
            let same = body.x.to_bits() == stepped.x.to_bits()
                && body.y.to_bits() == stepped.y.to_bits()
                && body.vx.to_bits() == stepped.vx.to_bits()
                && body.vy.to_bits() == stepped.vy.to_bits();
            assert!(!same, "{label} asleep={asleep} must move on step: {body:?}");
            assert!(
                !skip,
                "{label} asleep={asleep} must not skip: {body:?} stepped={stepped:?}"
            );
            assert_eq!(skip, same, "{label} asleep={asleep} parity");
        }
        for &x in &[f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let edge = Body::new(x, 0.5, 0.0, 0.0, 0.5, 0.5);
            agrees(edge, false, &cfg, "non-finite x");
            agrees(edge, true, &cfg, "non-finite x");
        }
        let neg_zero_x = Body::new(-0.0, 0.5, 0.0, 0.0, 0.5, 0.5);
        agrees(neg_zero_x, false, &cfg, "neg-zero x");
        agrees(neg_zero_x, true, &cfg, "neg-zero x");
        let signed_zero = Body::new(1.0, -0.0, 0.0, 0.0, 0.5, 0.0);
        agrees(signed_zero, false, &cfg, "signed-zero y/hy");
        agrees(signed_zero, true, &cfg, "signed-zero y/hy");
        let large_hy = Body::new(0.0, 100_000.0, 0.0, 0.0, 0.5, 100_000.0);
        agrees(large_hy, false, &cfg, "large hy");
        agrees(large_hy, true, &cfg, "large hy");
    }

    #[test]
    fn lod_mixed_1k_far_skip_wakes_100() {
        // 1k-agent part of mixed-1k-10k settled: far, still, grounded (LodWorld helpers, generic N).
        let cfg = LodConfig::default();
        assert!(classify(Tier::Capsule, DIST_UNKNOWN_MM, &cfg) == Tier::Capsule);
        const N: usize = 1_000;
        let bodies = vec![Body::new(1.0, 0.5, 0.0, 0.0, 0.5, 0.5); N];
        let mut skipped = 0_usize;
        for b in bodies.iter() {
            if static_sleep_skip(b, Tier::Capsule, false, &cfg) {
                skipped += 1;
            }
        }
        assert_eq!(skipped, N, "1k far still must all skip");
        let islands = crate::islands::build_islands(N, &[]);
        let mut v2 = vec![0.0f32; N];
        let (mut timers, mut asleep) = (vec![0u32; N], vec![false; N]);
        crate::islands::update_sleep(N, &islands, &v2, &mut timers, &mut asleep, cfg.sleep);
        assert!(!asleep[0], "hysteresis alone needs 60 steps, not one");
        let before = crate::rigid::state_hash(&bodies);
        let mut naive = bodies.clone();
        crate::rigid::integrate(&mut naive, FIXED_DT);
        assert_eq!(
            crate::rigid::state_hash(&naive),
            before,
            "idempotent rest keeps hash"
        );
        for (i, b) in bodies.iter().enumerate() {
            if static_sleep_skip(b, Tier::Capsule, false, &cfg) {
                asleep[i] = true;
                timers[i] = cfg.sleep.sleep_steps;
            }
        }
        crate::islands::update_sleep(N, &islands, &v2, &mut timers, &mut asleep, cfg.sleep);
        assert!(asleep.iter().all(|&s| s), "all 1k sleep in one step");
        let joined = crate::islands::build_islands(N, &[(0, 1)]);
        v2[0] = cfg.sleep.wake_v2 + 1.0;
        crate::islands::update_sleep(N, &joined, &v2, &mut timers, &mut asleep, cfg.sleep);
        assert!(!asleep[0] && !asleep[1], "kick wakes its island");
        assert!(asleep[2], "rest stays asleep");
    }

    #[test]
    fn lod_full_tier_never_sleeps_and_moves_hash() {
        let mut w = LodWorld::new_army(13, LodConfig::default());
        w.update_tiers(&tiers(1_000)).expect("near");
        assert_eq!(w.full_count(), SOLDIERS);
        let pair: Vec<(usize, usize)> = vec![(0, 1)];
        let cfg = w.cfg.sleep;
        for _ in 0..cfg.sleep_steps + 1 {
            w.step(FIXED_DT, &pair).expect("step");
        }
        assert!(
            !w.is_asleep(0).expect("slot 0"),
            "full tier steps even at rest"
        );
        let near_hash = w.state_hash();
        w.update_tiers(&tiers(DIST_UNKNOWN_MM)).expect("far");
        w.step(FIXED_DT, &pair).expect("step");
        assert_ne!(w.state_hash(), near_hash, "tier flip must move the digest");
    }

    /// The 500 + 64 army with the first half near: a worst-case-shaped mix of
    /// capsules and bones, tiers already applied.
    fn half_near_army() -> LodWorld {
        let mut w = LodWorld::new_army(2_040, LodConfig::default());
        let mut dist = vec![DIST_UNKNOWN_MM; SOLDIERS];
        for d in dist.iter_mut().take(SOLDIERS / 2) {
            *d = 1_000;
        }
        w.update_tiers(&dist).expect("tiers");
        w
    }

    #[test]
    fn lod_army_500_64_hash_x2() {
        let mut a = half_near_army();
        let mut b = half_near_army();
        assert_eq!(a.full_count(), SOLDIERS / 2);
        for _ in 0..10 {
            a.step(FIXED_DT, &[]).expect("step a");
            b.step(FIXED_DT, &[]).expect("step b");
        }
        assert_eq!(a.state_hash(), b.state_hash(), "same seed, same hash x2");
        a.step(FIXED_DT, &[]).expect("step a");
        assert_eq!(a.state_hash(), {
            b.step(FIXED_DT, &[]).expect("step b");
            b.state_hash()
        });
    }

    #[test]
    #[ignore = "timing: run under proof --perf"]
    #[expect(
        clippy::disallowed_methods,
        reason = "timing bounds the test slice only, never sim state"
    )]
    #[expect(
        clippy::disallowed_types,
        reason = "timing bounds the test slice only, never sim state"
    )]
    fn lod_army_500_64_under_4ms() {
        let mut a = half_near_army();
        for _ in 0..10 {
            a.step(FIXED_DT, &[]).expect("step a");
        }
        let start = std::time::Instant::now();
        a.step(FIXED_DT, &[]).expect("step a");
        let ns = start.elapsed().as_nanos();
        assert!(
            ns < 4_000_000,
            "500 + 64 physics slice over budget: {ns}ns > 4ms"
        );
    }

    #[test]
    fn lod_fixture_seed_separates_substreams() {
        let a = LodWorld::new_army(2_040, LodConfig::default());
        // `Rng::new` forces the low bit, so seeds `2k`/`2k + 1` share a
        // stream by design: separate with seeds that differ above bit 0.
        let b = LodWorld::new_army(9_001, LodConfig::default());
        assert_ne!(a.state_hash(), b.state_hash(), "seeds must separate");
        let c = LodWorld::new_army(2_040, LodConfig::default());
        assert_eq!(a.state_hash(), c.state_hash(), "same seed, same bytes");
    }
}
