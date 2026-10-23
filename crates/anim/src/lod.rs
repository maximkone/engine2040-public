//! lod: animation LOD update-rate tiers by distance and screen size.
//!
//! Owner: `engine-anim`; status: tier table plus the promotion rebuild.
//! UNLOCKS: the AM18 crowd follow-up (beside this file, lands after) plus
//! the arena duel graph tick consume [`assign_tiers`] to gate eval and
//! show each held pose through a [`LodSlot`], so a tier change or a strobe
//! hold never pops; no crowd handoff to VG4 in this wave. Units: meters,
//! screen fraction `0..=1`, ticks at 60 Hz, degrees for the promotion cap.
//! Determinism: comparisons plus `total_cmp` order only, ties by fighter
//! index, no clock; the rebuild is arithmetic over the same two poses, so
//! one tape shows the same pose twice. Errors: [`LodError`]. Budget: at
//! most [`MAX_FIGHTERS`] lanes over stack indices, no heap per call in the
//! table; a rebuild that still closes a gap blends one pose.
//!
//! A committed traversal warp owns its ticks: [`WarpWindow`] pins every one
//! of them to [`Tier::Full`], because the landing tick must carry the warped
//! root and a strobe hold would land it off the plan edge.

use super::contract::LocalPose;
use super::pose::blend_poses;
use super::traversal::TraversalPlan;

/// Largest fighter count in one LOD table; bounds the stack sort.
pub const MAX_FIGHTERS: usize = 64;
/// Neutral alias for the lane cap: new modules name this, while the genre
/// ledger keeps the arena-sized name in this file (GE2).
pub const MAX_ACTORS: usize = MAX_FIGHTERS;
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

/// Ticks one committed traversal warp owns: the plan's `duration_ticks` from
/// the commit tick, landing on the last of them. During the window the warped
/// root must be evaluated every tick, so [`tier_for`](Self::tier_for) answers
/// [`Tier::Full`]: a strobe hold would leave an older root on the landing tick
/// and miss the edge by the held ticks' travel. Empty for a
/// [`TraverseAction::None`](super::traversal::TraverseAction::None) plan.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WarpWindow {
    start_tick: u64,
    end_tick: u64,
}

impl WarpWindow {
    /// Window covering `plan`'s tick budget from `start_tick`, end exclusive.
    /// Pure table read: the same plan and tick give the same window twice.
    pub fn from_plan(plan: &TraversalPlan, start_tick: u64) -> Self {
        Self {
            start_tick,
            end_tick: start_tick.saturating_add(u64::from(plan.duration_ticks)),
        }
    }

    /// First tick the warp owns.
    pub fn start_tick(&self) -> u64 {
        self.start_tick
    }

    /// First tick after the warp (exclusive).
    pub fn end_tick(&self) -> u64 {
        self.end_tick
    }

    /// Landing tick: the last tick the warp owns, the root sample the edge
    /// tolerance is measured on. `None` for an empty window.
    pub fn landing_tick(&self) -> Option<u64> {
        if self.end_tick > self.start_tick {
            Some(self.end_tick - 1)
        } else {
            None
        }
    }

    /// True while the warp owns `tick`; every such tick is a no-strobe tick.
    pub fn contains(&self, tick: u64) -> bool {
        tick >= self.start_tick && tick < self.end_tick
    }

    /// Tier to record on `tick`: [`Tier::Full`] inside the window, `tier`
    /// outside it. A caller gates eval on the answer, so the window never
    /// holds a pose.
    pub fn tier_for(&self, tick: u64, tier: Tier) -> Tier {
        if self.contains(tick) {
            Tier::Full
        } else {
            tier
        }
    }
}

/// Nominal largest joint rotation one rebuild tick steps, in degrees. The
/// blend weight comes from the quaternion chord of this angle, which lands
/// the real step a hair past it (about 4.03 deg at a half-turn gap) and so
/// stays under the 5 deg/tick no-pop bar. The rebuild only walks a gap that
/// is wider than this; a narrower gap lands in one tick.
pub const PROMOTION_STEP_DEG: f32 = 4.0;
/// Largest joint translation one rebuild tick steps, in meters.
pub const PROMOTION_STEP_M: f32 = 0.05;

/// Chord of [`PROMOTION_STEP_DEG`] on the unit-quaternion sphere:
/// `2 * sin(1 deg)`, because a quaternion sits at half the joint angle.
const PROMOTION_CHORD: f32 = 0.034_905;

/// One fighter's LOD state: the assigned tier, the pose last shown and the
/// rebuild that closes the gap a coarse tier left behind.
///
/// Per tick: [`begin`](Self::begin) the tier [`assign_tiers`] reports, then
/// show the last full evaluation with [`rebuild`](Self::rebuild). The slot
/// copies a full-rate target exactly, so the nearest eight stay bit-equal to
/// full rate, and walks a wider gap at the step cap, so a promotion or a
/// strobe hold never pops. A warp-window tick goes in through
/// [`begin_pinned`](Self::begin_pinned) instead: it records [`Tier::Full`] and
/// shows that tick's evaluation exactly, so a traversal landing never reads a
/// held pose. [`take_reseed`](Self::take_reseed) asks the caller to re-seed
/// spring bones at the shown pose once per promotion.
#[derive(Clone, Debug)]
pub struct LodSlot {
    tier: Tier,
    shown: LocalPose,
    rebuilding: bool,
    reseed_springs: bool,
    pinned: bool,
}

impl LodSlot {
    /// Empty slot: far tier, no shown pose, settled.
    pub fn new() -> Self {
        Self {
            tier: Tier::Far,
            shown: LocalPose {
                positions: Vec::new(),
                rotations: Vec::new(),
            },
            rebuilding: false,
            reseed_springs: false,
            pinned: false,
        }
    }

    /// Tier recorded by the last [`begin`](Self::begin).
    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// Pose last shown; it holds unchanged while a coarse tier skips eval.
    pub fn shown(&self) -> &LocalPose {
        &self.shown
    }

    /// True while a rebuild still closes a wider-than-one-step gap.
    pub fn rebuilding(&self) -> bool {
        self.rebuilding
    }

    /// Record this tick's tier. A promotion (a shorter eval period than the
    /// recorded tier) asks for a spring re-seed: the coarse tier froze the
    /// shown pose, so springs must start from it rather than from velocity
    /// the skipped ticks never advanced. A demotion or a hold leaves the
    /// shown pose alone; a slot with no shown pose has nothing to rebuild.
    pub fn begin(&mut self, tier: Tier) {
        self.begin_pinned(tier, false);
    }

    /// Record this tick's tier, with `pinned` marking a
    /// [`WarpWindow`] tick. A pinned tick records [`Tier::Full`] whatever the
    /// distance tier reads, and [`rebuild`](Self::rebuild) copies that tick's
    /// evaluation exactly: an exact landing has no room for a strobe hold or
    /// a rebuild step, and the warp itself places the root.
    pub fn begin_pinned(&mut self, tier: Tier, pinned: bool) {
        let effective = if pinned { Tier::Full } else { tier };
        if effective.period_ticks() < self.tier.period_ticks() && !self.shown.positions.is_empty() {
            self.rebuilding = true;
            self.reseed_springs = true;
        }
        self.tier = effective;
        self.pinned = pinned;
    }

    /// Take the spring re-seed request: true once after a promotion.
    pub fn take_reseed(&mut self) -> bool {
        core::mem::take(&mut self.reseed_springs)
    }

    /// Show `target`, the fighter's last full evaluation, this tick. A pinned
    /// warp-window tick copies it exactly. A full rate slot that is not
    /// rebuilding copies it exactly; every other slot copies it when the gap
    /// fits in one step and otherwise walks at most
    /// [`PROMOTION_STEP_DEG`] and [`PROMOTION_STEP_M`] per joint toward it.
    /// Returns true when the slot is settled on `target`.
    pub fn rebuild(&mut self, target: &LocalPose) -> bool {
        if self.pinned {
            // No strobe hold, no rebuild step: the landing root is this
            // tick's evaluation.
            self.shown = target.clone();
            self.rebuilding = false;
            return true;
        }
        let same_shape = self.shown.positions.len() == target.positions.len()
            && self.shown.rotations.len() == target.rotations.len();
        if !same_shape {
            self.shown = target.clone();
            self.rebuilding = false;
            return true;
        }
        if !self.rebuilding && self.tier == Tier::Full {
            self.shown = target.clone();
            return true;
        }
        let weight = promotion_weight(&self.shown, target);
        if weight >= 1.0 {
            self.shown = target.clone();
            self.rebuilding = false;
            true
        } else {
            self.shown = blend_poses(&self.shown, target, weight);
            self.rebuilding = true;
            false
        }
    }
}

impl Default for LodSlot {
    fn default() -> Self {
        Self::new()
    }
}

/// Blend weight that keeps every joint gap within one step: the smallest
/// ratio over the shortest-arc rotation chord and the translation length,
/// capped at 1.0 (a gap that already fits). Arithmetic only: the chord is a
/// monotone stand-in for the angle, so no transcendental enters the tick.
fn promotion_weight(shown: &LocalPose, target: &LocalPose) -> f32 {
    let joints = shown
        .positions
        .len()
        .min(shown.rotations.len())
        .min(target.positions.len())
        .min(target.rotations.len());
    let mut weight = 1.0f32;
    for joint in 0..joints {
        let a = shown.rotations[joint];
        let b = target.rotations[joint];
        let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
        let sign = if dot < 0.0 { -1.0 } else { 1.0 };
        let dx = a[0] - sign * b[0];
        let dy = a[1] - sign * b[1];
        let dz = a[2] - sign * b[2];
        let dw = a[3] - sign * b[3];
        let chord = (dx * dx + dy * dy + dz * dz + dw * dw).sqrt();
        if chord > PROMOTION_CHORD {
            weight = weight.min(PROMOTION_CHORD / chord);
        }
        let p = shown.positions[joint];
        let q = target.positions[joint];
        let mx = q[0] - p[0];
        let my = q[1] - p[1];
        let mz = q[2] - p[2];
        let span = (mx * mx + my * my + mz * mz).sqrt();
        if span > PROMOTION_STEP_M {
            weight = weight.min(PROMOTION_STEP_M / span);
        }
    }
    weight
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Quaternion for a yaw of `deg` degrees: half-angle `[0, sin, 0, cos]`.
    #[expect(
        clippy::disallowed_methods,
        reason = "test fixture: builds quats; the rebuild itself uses arithmetic only"
    )]
    fn yaw_quat(deg: f32) -> [f32; 4] {
        let half = 0.5 * deg.to_radians();
        [0.0, half.sin(), 0.0, half.cos()]
    }

    /// Full-rate fighter pose at `tick`: four joints, every joint moving
    /// (fastest 2 deg and 8 mm a tick), so a strobe hold leaves a real gap.
    fn fine_pose(tick: u64) -> LocalPose {
        let t = f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
        LocalPose {
            positions: vec![
                [0.008 * t, 1.0, 0.0],
                [0.0, 1.2, 0.0],
                [0.0, 1.4, 0.0],
                [0.0, 0.9, 0.0],
            ],
            rotations: vec![
                yaw_quat(t),
                yaw_quat(0.5 * t),
                yaw_quat(-t),
                yaw_quat(2.0 * t),
            ],
        }
    }

    /// What a far tier shows: IK and springs are off, so the spring tip
    /// droops 18 cm and its joint sits 20 deg off the fine pose.
    fn coarse_pose(tick: u64) -> LocalPose {
        let mut pose = fine_pose(tick);
        let t = f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
        pose.positions[3][1] -= 0.18;
        pose.rotations[3] = yaw_quat(2.0 * t + 20.0);
        pose
    }

    /// Distance of fighter zero in the approach tape: 60 m to 10 m over 30
    /// ticks, one tick per step.
    fn approach_dist(tick: u64) -> f32 {
        let t = f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
        60.0 - 50.0 * t / 30.0
    }

    /// Largest joint rotation between two poses, degrees.
    #[expect(
        clippy::disallowed_methods,
        reason = "test oracle: measures the angle; the rebuild itself is arithmetic only"
    )]
    fn rotation_step_deg(a: &LocalPose, b: &LocalPose) -> f32 {
        let joints = a.rotations.len().min(b.rotations.len());
        let mut worst = 0.0f32;
        for joint in 0..joints {
            let q = a.rotations[joint];
            let r = b.rotations[joint];
            let dot = (q[0] * r[0] + q[1] * r[1] + q[2] * r[2] + q[3] * r[3])
                .abs()
                .min(1.0);
            let deg = 2.0 * dot.acos().to_degrees();
            if deg > worst {
                worst = deg;
            }
        }
        worst
    }

    /// Largest joint translation between two poses, meters.
    fn position_step_m(a: &LocalPose, b: &LocalPose) -> f32 {
        let joints = a.positions.len().min(b.positions.len());
        let mut worst = 0.0f32;
        for joint in 0..joints {
            let dx = b.positions[joint][0] - a.positions[joint][0];
            let dy = b.positions[joint][1] - a.positions[joint][1];
            let dz = b.positions[joint][2] - a.positions[joint][2];
            let m = (dx * dx + dy * dy + dz * dz).sqrt();
            if m > worst {
                worst = m;
            }
        }
        worst
    }

    /// The row's no-pop bar: one fighter closes 60 m to 10 m over 30 ticks
    /// through all four tiers while 63 others fill the tape, and no tick
    /// moves a joint past 5 deg or a hand span past the step cap. The same
    /// tape pinned to the 8 nearest still matches full rate bit for bit, and
    /// a caller that copies the eval instead of rebuilding it pops, so the
    /// test fails without the rebuild.
    #[test]
    fn lod_promotion_rebuild_has_no_pop() {
        let mut slots: Vec<LodSlot> = (0..MAX_FIGHTERS).map(|_| LodSlot::new()).collect();
        let mut targets: Vec<LocalPose> = (0..MAX_FIGHTERS)
            .map(|_| LocalPose {
                positions: Vec::new(),
                rotations: Vec::new(),
            })
            .collect();
        let mut tiers = vec![Tier::Far; MAX_FIGHTERS];
        let probe = FULL_RATE_NEAREST;
        let mut reseeds = 0u32;
        let mut seen = [false; 4];
        let mut max_deg = 0.0f32;
        let mut max_m = 0.0f32;
        let mut naive_deg = 0.0f32;
        let mut naive_m = 0.0f32;
        let mut naive_prev: Option<LocalPose> = None;
        let mut prev_shown: Option<LocalPose> = None;
        for tick in 0..=30u64 {
            let mut dists = vec![0.0f32; MAX_FIGHTERS];
            for (i, d) in dists.iter_mut().enumerate() {
                let idx = f32::from(u16::try_from(i).unwrap_or(u16::MAX));
                *d = if i < FULL_RATE_NEAREST {
                    5.0
                } else if i == probe {
                    approach_dist(tick)
                } else {
                    90.0 + idx
                };
            }
            let screens = vec![0.002f32; MAX_FIGHTERS];
            assign_tiers(&dists, &screens, &mut tiers).expect("approach tape assigns");
            for i in 0..MAX_FIGHTERS {
                slots[i].begin(tiers[i]);
                if slots[i].take_reseed() {
                    reseeds += 1;
                }
                if tiers[i].updates_this_tick(tick, i) {
                    let next = if tiers[i] == Tier::Far {
                        coarse_pose(tick)
                    } else {
                        fine_pose(tick)
                    };
                    if i == probe {
                        // The caller without the rebuild shows each fresh eval
                        // as it comes; these are its per-tick jumps.
                        if let Some(prev) = naive_prev.as_ref() {
                            naive_deg = naive_deg.max(rotation_step_deg(prev, &next));
                            naive_m = naive_m.max(position_step_m(prev, &next));
                        }
                        naive_prev = Some(next.clone());
                    }
                    targets[i] = next;
                }
                slots[i].rebuild(&targets[i]);
            }
            if let Some(prev) = prev_shown.as_ref() {
                let shown = slots[probe].shown();
                max_deg = max_deg.max(rotation_step_deg(prev, shown));
                max_m = max_m.max(position_step_m(prev, shown));
            }
            for (i, slot) in slots.iter().enumerate().take(FULL_RATE_NEAREST) {
                assert_eq!(
                    slot.shown(),
                    &fine_pose(tick),
                    "nearest {i} matches full rate at tick {tick}"
                );
                assert!(
                    !slot.rebuilding(),
                    "nearest {i} never rebuilds at tick {tick}"
                );
            }
            match slots[probe].tier() {
                Tier::Far => seen[0] = true,
                Tier::Quarter => seen[1] = true,
                Tier::Half => seen[2] = true,
                Tier::Full => seen[3] = true,
            }
            prev_shown = Some(slots[probe].shown().clone());
        }
        assert!(
            seen.iter().all(|&crossed| crossed),
            "the approach crosses all four tiers, saw {seen:?}"
        );
        assert_eq!(
            reseeds, 3,
            "one spring re-seed per promotion, got {reseeds}"
        );
        assert!(
            max_deg < 5.0,
            "a promoted fighter stepped {max_deg} deg in one tick"
        );
        assert!(
            max_m < 0.050_1,
            "a promoted fighter stepped {max_m} m in one tick"
        );
        assert!(
            naive_deg > 5.0 && naive_m > PROMOTION_STEP_M,
            "a caller without the rebuild must pop, got {naive_deg} deg and {naive_m} m"
        );
        assert!(
            max_deg < naive_deg && max_m < naive_m,
            "the rebuild is what removes the pop: {max_deg} vs {naive_deg} deg"
        );
    }

    /// The re-seed contract: exactly one request per promotion, consumed by
    /// the caller, and nothing on a demotion or a hold.
    #[test]
    fn lod_reseed_fires_once_per_promotion() {
        let pose = fine_pose(3);
        let mut slot = LodSlot::new();
        slot.begin(Tier::Far);
        assert!(slot.rebuild(&pose), "the first show settles");
        assert!(!slot.take_reseed(), "no stale pose, nothing to re-seed");
        slot.begin(Tier::Half);
        assert!(slot.take_reseed(), "a promotion asks for a re-seed once");
        assert!(!slot.take_reseed(), "the request is consumed");
        slot.begin(Tier::Full);
        assert!(slot.take_reseed(), "the next promotion asks again");
        assert!(slot.rebuild(&pose), "no gap means one tick settles");
        assert!(!slot.rebuilding());
        slot.begin(Tier::Quarter);
        assert!(!slot.take_reseed(), "a demotion asks for nothing");
        assert!(!slot.rebuilding(), "a demotion leaves the pose alone");
    }

    /// Timing report for the row's 64-fighter bar. The LOD slice only: tier
    /// assign plus the per-fighter show; pose eval runs at the tier rate and
    /// is not owned here. No wall clock enters the tick (test-only report).
    #[test]
    #[ignore = "timing: 64-fighter 2 ms bar, run with --ignored --release"]
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "wall clock in a test-only timing report, never in the tick"
    )]
    fn lod_sixty_four_table_tick_under_two_ms() {
        let mut dists = vec![0.0f32; MAX_FIGHTERS];
        for (i, d) in dists.iter_mut().enumerate() {
            let idx = f32::from(u16::try_from(i).unwrap_or(u16::MAX));
            *d = 4.0 + 1.5 * idx;
        }
        let screens = vec![0.002f32; MAX_FIGHTERS];
        let mut tiers = vec![Tier::Far; MAX_FIGHTERS];
        let mut slots: Vec<LodSlot> = (0..MAX_FIGHTERS).map(|_| LodSlot::new()).collect();
        let target = fine_pose(17);
        let ticks = 5_000u32;
        let begin = std::time::Instant::now();
        for _ in 0..ticks {
            assign_tiers(&dists, &screens, &mut tiers).expect("timing tape assigns");
            for i in 0..MAX_FIGHTERS {
                slots[i].begin(tiers[i]);
                slots[i].rebuild(&target);
            }
            std::hint::black_box(&slots[MAX_FIGHTERS - 1]);
        }
        let ns = begin.elapsed().as_nanos() / u128::from(ticks);
        println!("lod: {ns} ns per 64-fighter tick (bar 2000000)");
        assert!(ns < 2_000_000, "{ns} ns past the 2 ms 64-fighter bar");
    }
}
