//! lod: animation LOD update-rate tiers by distance and screen size.
//!
//! Owner: `engine-anim`; status: partial table only. UNLOCKS: the AM18
//! crowd follow-up (beside this file, lands after) plus the arena duel
//! graph tick consume [`assign_tiers`] to gate eval and hold far poses;
//! no crowd handoff to VG4 in this wave. Units: meters, screen fraction
//! `0..=1`, ticks at 60 Hz. Determinism: comparisons plus `total_cmp`
//! order only, ties by fighter index, no clock. Errors: [`LodError`].
//! Budget: at most [`MAX_FIGHTERS`] lanes over stack indices, no heap
//! per call; table work only, eval excluded.

/// Largest fighter count in one LOD table; bounds the stack sort.
pub const MAX_FIGHTERS: usize = 64;
/// Nearest fighters kept at full rate whatever their distance reads.
pub const FULL_RATE_NEAREST: usize = 8;
/// Below this distance (m) a fighter stays full rate.
pub const NEAR_DIST_M: f32 = 12.0;
/// Below this distance (m) a fighter stays at half rate or better.
pub const MID_DIST_M: f32 = 30.0;
/// At or past this distance (m) a small fighter holds the far tier.
pub const FAR_DIST_M: f32 = 60.0;
/// At or above this screen fraction a fighter stays full rate.
pub const BIG_SCREEN: f32 = 0.04;
/// At or above this screen fraction a mid fighter stays half rate.
pub const SMALL_SCREEN: f32 = 0.01;

/// Update-rate tier: how often a fighter re-evaluates its pose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    /// Every tick, IK and springs on.
    Full,
    /// Every 2 ticks, IK and springs on.
    Half,
    /// Every 4 ticks, IK and springs on.
    Quarter,
    /// Every 8 ticks held, IK and springs off.
    Far,
}

impl Tier {
    /// Ticks between evals: 1, 2, 4, 8.
    pub fn period_ticks(self) -> u64 {
        match self {
            Tier::Full => 1,
            Tier::Half => 2,
            Tier::Quarter => 4,
            Tier::Far => 8,
        }
    }

    /// True unless the tier is [`Tier::Far`].
    pub fn ik_on(self) -> bool {
        !matches!(self, Tier::Far)
    }

    /// True unless the tier is [`Tier::Far`].
    pub fn springs_on(self) -> bool {
        !matches!(self, Tier::Far)
    }

    /// True when `slot` updates on `tick`; slots stagger far tiers.
    pub fn updates_this_tick(self, tick: u64, slot: usize) -> bool {
        let period = self.period_ticks();
        if period <= 1 {
            return true;
        }
        debug_assert!(u64::try_from(slot).is_ok());
        let slot_u = u64::try_from(slot).unwrap_or(u64::MAX);
        (tick.wrapping_add(slot_u) % period) == 0
    }
}

/// Bounded LOD-input failures: typed, never panics.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LodError {
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
    /// Distance below zero; value carried.
    NegativeDistance(f32),
    /// Screen fraction outside `0..=1`; value carried.
    BadScreen(f32),
    /// Lane count mismatch; expected plus got.
    LengthMismatch { expected: usize, got: usize },
    /// Entry count past [`MAX_FIGHTERS`]; value carried.
    TooManyEntries(usize),
}

impl core::fmt::Display for LodError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LodError::NonFinite(lane) => write!(f, "lod: non-finite float in {lane}"),
            LodError::NegativeDistance(v) => write!(f, "lod: distance {v} below zero"),
            LodError::BadScreen(v) => write!(f, "lod: screen {v} outside 0..=1"),
            LodError::LengthMismatch { expected, got } => {
                write!(f, "lod: lane count {got} differs from {expected}")
            }
            LodError::TooManyEntries(n) => {
                write!(f, "lod: {n} fighters past {MAX_FIGHTERS}")
            }
        }
    }
}

impl std::error::Error for LodError {}

/// Base tier from distance (m) plus screen fraction, before the
/// nearest-eight rule. Big screens keep fighters full rate far away.
pub fn tier_for(distance_m: f32, screen_frac: f32) -> Result<Tier, LodError> {
    if !distance_m.is_finite() {
        return Err(LodError::NonFinite("distance_m"));
    }
    if !screen_frac.is_finite() {
        return Err(LodError::NonFinite("screen_frac"));
    }
    if distance_m < 0.0 {
        return Err(LodError::NegativeDistance(distance_m));
    }
    if !(0.0..=1.0).contains(&screen_frac) {
        return Err(LodError::BadScreen(screen_frac));
    }
    if distance_m < NEAR_DIST_M || screen_frac >= BIG_SCREEN {
        Ok(Tier::Full)
    } else if distance_m < MID_DIST_M || screen_frac >= SMALL_SCREEN {
        Ok(Tier::Half)
    } else if distance_m < FAR_DIST_M {
        Ok(Tier::Quarter)
    } else {
        Ok(Tier::Far)
    }
}

/// Fill `out` with one tier per fighter, then force the
/// [`FULL_RATE_NEAREST`] nearest to [`Tier::Full`]. Slices in, slice
/// out: no heap. Ties break by fighter index, so one tape assigns
/// identically twice.
pub fn assign_tiers(
    distances_m: &[f32],
    screen_fracs: &[f32],
    out: &mut [Tier],
) -> Result<(), LodError> {
    if distances_m.len() != screen_fracs.len() {
        return Err(LodError::LengthMismatch {
            expected: distances_m.len(),
            got: screen_fracs.len(),
        });
    }
    if out.len() != distances_m.len() {
        return Err(LodError::LengthMismatch {
            expected: distances_m.len(),
            got: out.len(),
        });
    }
    if distances_m.len() > MAX_FIGHTERS {
        return Err(LodError::TooManyEntries(distances_m.len()));
    }
    for (i, (&d, &s)) in distances_m.iter().zip(screen_fracs.iter()).enumerate() {
        if !d.is_finite() {
            return Err(LodError::NonFinite("distances_m"));
        }
        if !s.is_finite() {
            return Err(LodError::NonFinite("screen_fracs"));
        }
        if d < 0.0 {
            return Err(LodError::NegativeDistance(d));
        }
        if !(0.0..=1.0).contains(&s) {
            return Err(LodError::BadScreen(s));
        }
        let _ = i;
    }
    for (i, slot) in out.iter_mut().enumerate() {
        // Validated above, so the lookup cannot fail.
        match tier_for(distances_m[i], screen_fracs[i]) {
            Ok(t) => *slot = t,
            Err(_) => *slot = Tier::Far,
        }
    }
    // Stack rank of fighter indices by distance, ties by index.
    let mut order = [0usize; MAX_FIGHTERS];
    for (rank, slot) in order.iter_mut().take(distances_m.len()).enumerate() {
        *slot = rank;
    }
    let (ranked, _) = order.split_at_mut(distances_m.len());
    ranked.sort_unstable_by(|&a, &b| {
        distances_m[a]
            .total_cmp(&distances_m[b])
            .then_with(|| a.cmp(&b))
    });
    let keep = FULL_RATE_NEAREST.min(out.len());
    for &fighter in ranked.iter().take(keep) {
        out[fighter] = Tier::Full;
    }
    Ok(())
}
