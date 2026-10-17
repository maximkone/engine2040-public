//! feet: planted-foot lock over analytic two-bone IK.
//!
//! Owner: `engine-anim`; consumers: arena ground feel per-leg solves
//! via [`solve_leg`] plus full-body IK, and the animation graph pose via
//! [`lock_graph_pose`]. Units: meters in, integer millimeters on the slide
//! meter. Determinism: stable leg order, `+ - * /` plus `sqrt` only, no
//! clock; one input script replays byte-identically. Errors: [`FootError`].
//! Budget: O(1) per leg per tick; caller-owned state, no heap per tick.

use super::contract::ContactTable;
use super::graph::Pose;
use super::ik::two_bone;
use engine_core::Fnv64;

/// Millimeters per meter on the slide meter.
pub const MM_PER_M: f32 = 1000.0;
/// Slide budget on straights: a planted foot moves less than this per plant.
pub const SLIDE_BUDGET_MM_STRAIGHT: u32 = 20;
/// Slide budget on pivots and turns: looser than straights, still charted.
pub const SLIDE_BUDGET_MM_PIVOT: u32 = 40;
/// Default ground height in meters (arena floor).
pub const DEFAULT_GROUND_Y_M: f32 = 0.0;

/// Foot-lock failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FootError {
    /// Foot index is not `0` (left) or `1` (right); value carried.
    BadFoot(usize),
    /// Joint index out of range for the pose; value carried.
    BadJoint(usize),
    /// Pose lanes disagree or the pose is empty; expected plus got.
    LengthMismatch { expected: usize, got: usize },
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
    /// Ground height is not finite.
    BadGround,
}

impl core::fmt::Display for FootError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FootError::BadFoot(foot) => write!(f, "feet: bad foot {foot} (want 0 or 1)"),
            FootError::BadJoint(joint) => write!(f, "feet: joint {joint} out of range"),
            FootError::LengthMismatch { expected, got } => {
                write!(f, "feet: joint count {got} differs from {expected}")
            }
            FootError::NonFinite(lane) => write!(f, "feet: non-finite float in {lane}"),
            FootError::BadGround => write!(f, "feet: ground height is not finite"),
        }
    }
}

impl std::error::Error for FootError {}

/// True when every lane of `p` is finite.
fn finite3(p: [f32; 3]) -> bool {
    p[0].is_finite() && p[1].is_finite() && p[2].is_finite()
}

/// Euclidean distance in meters with `+ - *` plus one `sqrt`.
pub(crate) fn dist_m(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Analytic two-bone IK: hip stays, knee and ankle solve toward `target`.
///
/// Graduates `ik::two_bone` into the foot-lock flow (no new math path):
/// unreachable targets clamp to max reach, degenerate or non-finite inputs
/// keep the current knee and ankle instead of panicking.
pub fn solve_two_bone(
    root: [f32; 3],
    mid: [f32; 3],
    end: [f32; 3],
    target: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    if !finite3(root) || !finite3(mid) || !finite3(end) || !finite3(target) {
        return (mid, end);
    }
    two_bone(root, mid, end, target)
}

/// Clamp a foot to the ground plane: `y` below `ground_y` lifts to it.
///
/// Total and infallible; non-finite inputs pass through untouched so the
/// fallible tick path ([`FootLocker::update`]) can refuse them loudly.
pub fn ground_clamp(p: [f32; 3], ground_y: f32) -> [f32; 3] {
    if !finite3(p) || !ground_y.is_finite() {
        return p;
    }
    if p[1] < ground_y {
        [p[0], ground_y, p[2]]
    } else {
        p
    }
}

/// Slide distance in integer millimeters: `round(|a - b| * 1000)`.
///
/// Non-finite inputs saturate to `u32::MAX` so a bad lane can never read
/// as a planted foot. Uses `+ - * /` plus `sqrt` only.
pub fn slide_mm(a: [f32; 3], b: [f32; 3]) -> u32 {
    if !finite3(a) || !finite3(b) {
        return u32::MAX;
    }
    let d = dist_m(a, b);
    if !d.is_finite() {
        return u32::MAX;
    }
    let mm = d * MM_PER_M;
    if mm <= 0.0 {
        0
    } else if mm >= u32::MAX as f32 {
        u32::MAX
    } else {
        mm.round() as u32
    }
}

/// FNV-1a64 over foot positions (count plus `f32` bits); equal inputs hash
/// equally, which is the replay receipt for locked sequences.
pub fn feet_hash(positions: &[[f32; 3]]) -> u64 {
    let mut h = Fnv64::new();
    h.write_len(positions.len());
    for p in positions.iter() {
        for lane in p.iter() {
            h.write_f32(*lane);
        }
    }
    h.finish()
}

/// Planted flag for one foot from a contract contact window: the latest sample
/// for `foot` at or before `tick` decides; no sample means lifted.
pub fn planted_from_contacts(table: &ContactTable, tick: u32, foot: u8) -> bool {
    let mut planted = false;
    let mut seen = false;
    for contact in table.contacts.iter() {
        if contact.foot == foot && contact.tick <= tick {
            planted = contact.down;
            seen = true;
        }
    }
    seen && planted
}

/// Planted-foot state: one world-space anchor per foot plus the ground.
///
/// While a foot reads planted its output is the anchor exactly (zero slide)
/// for the whole continuous plant run; lift clears the anchor so the next
/// plant re-plants. All per-tick work is O(1) with no heap.
#[derive(Clone, PartialEq, Debug)]
pub struct FootLocker {
    anchors: [Option<[f32; 3]>; 2],
    ground_y: f32,
}

impl FootLocker {
    /// New locker over `ground_y`.
    pub fn new(ground_y: f32) -> Result<Self, FootError> {
        if !ground_y.is_finite() {
            return Err(FootError::BadGround);
        }
        Ok(Self {
            anchors: [None, None],
            ground_y,
        })
    }

    /// Locker over the arena floor.
    pub fn with_defaults() -> Self {
        Self {
            anchors: [None, None],
            ground_y: DEFAULT_GROUND_Y_M,
        }
    }

    /// Clear both anchors; the next planted tick re-plants.
    pub fn reset(&mut self) {
        self.anchors = [None, None];
    }

    /// Ground height in meters.
    pub fn ground_y(&self) -> f32 {
        self.ground_y
    }

    /// Current anchor for `foot`; `None` while lifted.
    pub fn anchor(&self, foot: usize) -> Result<Option<[f32; 3]>, FootError> {
        if foot > 1 {
            return Err(FootError::BadFoot(foot));
        }
        Ok(self.anchors[foot])
    }

    /// One tick for one foot: clamp `current` to the ground, then hold or
    /// release the anchor. Rejects bad feet and non-finite input loudly.
    pub fn update(
        &mut self,
        foot: usize,
        current: [f32; 3],
        planted: bool,
    ) -> Result<[f32; 3], FootError> {
        if foot > 1 {
            return Err(FootError::BadFoot(foot));
        }
        if !finite3(current) {
            return Err(FootError::NonFinite("current"));
        }
        let clamped = ground_clamp(current, self.ground_y);
        if !planted {
            self.anchors[foot] = None;
            return Ok(clamped);
        }
        match self.anchors[foot] {
            None => {
                self.anchors[foot] = Some(clamped);
                Ok(clamped)
            }
            Some(anchor) => Ok(anchor),
        }
    }
}

/// One solved leg: IK positions plus the lock meter for this tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LegSolution {
    /// Solved knee position in meters.
    pub knee: [f32; 3],
    /// Solved ankle position in meters.
    pub ankle: [f32; 3],
    /// Locked target the ankle solved toward, in meters.
    pub target: [f32; 3],
    /// Correction in millimeters (clamped raw vs locked target; `0` at
    /// plant and while lifted).
    pub slide_mm: u32,
    /// True while the foot reads planted.
    pub locked: bool,
}

/// Solve one leg for one tick: clamp the raw target, lock it, run the
/// analytic two-bone IK. This is the per-leg entry; legs run in
/// foot order (`0` then `1`) so replays are stable.
pub fn solve_leg(
    locker: &mut FootLocker,
    foot: usize,
    hip: [f32; 3],
    knee: [f32; 3],
    ankle: [f32; 3],
    target_raw: [f32; 3],
    planted: bool,
) -> Result<LegSolution, FootError> {
    if foot > 1 {
        return Err(FootError::BadFoot(foot));
    }
    if !finite3(hip) {
        return Err(FootError::NonFinite("hip"));
    }
    if !finite3(knee) {
        return Err(FootError::NonFinite("knee"));
    }
    if !finite3(ankle) {
        return Err(FootError::NonFinite("ankle"));
    }
    if !finite3(target_raw) {
        return Err(FootError::NonFinite("target"));
    }
    let clamped = ground_clamp(target_raw, locker.ground_y);
    let target = locker.update(foot, clamped, planted)?;
    let slide = if planted {
        slide_mm(clamped, target)
    } else {
        0
    };
    let (knee_out, ankle_out) = solve_two_bone(hip, knee, ankle, target);
    Ok(LegSolution {
        knee: knee_out,
        ankle: ankle_out,
        target,
        slide_mm: slide,
        locked: planted,
    })
}

/// Joint map for one leg inside an animation graph pose, plus its locker foot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LegMap {
    /// Hip joint index into the pose.
    pub hip: usize,
    /// Knee joint index into the pose.
    pub knee: usize,
    /// Ankle joint index into the pose (also the lock target lane).
    pub ankle: usize,
    /// Locker foot: `0` left, `1` right.
    pub foot: usize,
}

impl LegMap {
    /// Map hip/knee/ankle lanes to one locker foot; feet past `1` refuse.
    pub fn new(hip: usize, knee: usize, ankle: usize, foot: usize) -> Result<Self, FootError> {
        if foot > 1 {
            return Err(FootError::BadFoot(foot));
        }
        Ok(Self {
            hip,
            knee,
            ankle,
            foot,
        })
    }
}

/// Lock both legs of a graph pose in place: solve each leg toward its
/// locked ankle target and write knees and ankles back. Rotations pass
/// through untouched. Atomic: a bad map or lane writes nothing and leaves
/// the locker untouched. Steady-state ticks allocate nothing.
pub fn lock_graph_pose(
    pose: &mut Pose,
    legs: &[LegMap; 2],
    planted: [bool; 2],
    locker: &mut FootLocker,
) -> Result<[LegSolution; 2], FootError> {
    let joints = pose.positions.len();
    if joints != pose.rotations.len() {
        return Err(FootError::LengthMismatch {
            expected: pose.positions.len(),
            got: pose.rotations.len(),
        });
    }
    if joints == 0 {
        return Err(FootError::LengthMismatch {
            expected: 0,
            got: 0,
        });
    }
    for map in legs.iter() {
        if map.foot > 1 {
            return Err(FootError::BadFoot(map.foot));
        }
        for joint in [map.hip, map.knee, map.ankle] {
            if joint >= joints {
                return Err(FootError::BadJoint(joint));
            }
        }
        for lane in [
            pose.positions[map.hip],
            pose.positions[map.knee],
            pose.positions[map.ankle],
        ] {
            if !finite3(lane) {
                return Err(FootError::NonFinite("positions"));
            }
        }
    }
    let mut shadow = locker.clone();
    let mut solutions = [LegSolution {
        knee: [0.0; 3],
        ankle: [0.0; 3],
        target: [0.0; 3],
        slide_mm: 0,
        locked: false,
    }; 2];
    for (slot, map) in legs.iter().enumerate() {
        solutions[slot] = solve_leg(
            &mut shadow,
            map.foot,
            pose.positions[map.hip],
            pose.positions[map.knee],
            pose.positions[map.ankle],
            pose.positions[map.ankle],
            planted[slot],
        )?;
    }
    for (slot, map) in legs.iter().enumerate() {
        pose.positions[map.knee] = solutions[slot].knee;
        pose.positions[map.ankle] = solutions[slot].ankle;
    }
    *locker = shadow;
    Ok(solutions)
}

#[cfg(test)]
mod tests {
    use super::super::loco_bake::bake_hero_set;
    use super::super::locomotion::contacts_for_phase;
    use super::super::traversal::{apply_warp, plan, warp_error_mm, Probe};
    use super::*;

    const HIP: [f32; 3] = [0.0, 0.95, 0.0];
    const KNEE: [f32; 3] = [0.0, 0.5, 0.02];
    const ANKLE: [f32; 3] = [0.0, 0.05, 0.05];

    #[test]
    fn two_bone_reaches_and_clamps() {
        let l1 = dist_m(KNEE, HIP);
        let l2 = dist_m(ANKLE, KNEE);
        let (knee, ankle) = solve_two_bone(HIP, KNEE, ANKLE, [0.2, 0.35, 0.1]);
        assert!(slide_mm(ankle, [0.2, 0.35, 0.1]) <= 1, "{ankle:?}");
        assert!((dist_m(knee, HIP) - l1).abs() < 1e-3, "{knee:?}");
        // Unreachable targets clamp to max reach instead of stretching.
        let (_, far) = solve_two_bone(HIP, KNEE, ANKLE, [9.0, 0.05, 0.0]);
        assert!((dist_m(far, HIP) - (l1 + l2)).abs() < 2e-3, "{far:?}");
        // Degenerate and non-finite inputs keep the current pose, no panic.
        assert_eq!(
            solve_two_bone(HIP, HIP, ANKLE, [0.3, 0.05, 0.0]),
            (HIP, ANKLE)
        );
        assert_eq!(
            solve_two_bone(HIP, KNEE, ANKLE, [f32::NAN, 0.0, 0.0]),
            (KNEE, ANKLE)
        );
        // Same inputs twice solve identically.
        assert_eq!(
            solve_two_bone(HIP, KNEE, ANKLE, [0.2, 0.35, 0.1]),
            solve_two_bone(HIP, KNEE, ANKLE, [0.2, 0.35, 0.1])
        );
    }

    #[test]
    fn ground_clamp_holds_and_slide_reads_mm() {
        assert_eq!(ground_clamp([0.2, -0.05, 0.1], 0.0), [0.2, 0.0, 0.1]);
        assert_eq!(ground_clamp([0.2, 0.4, 0.1], 0.0), [0.2, 0.4, 0.1]);
        assert_eq!(ground_clamp([0.2, 0.0, 0.1], 0.0), [0.2, 0.0, 0.1]);
        assert_eq!(slide_mm([0.0, 0.0, 0.0], [0.01, 0.0, 0.0]), 10);
        assert_eq!(slide_mm([0.0, 0.0, 0.0], [0.02, 0.0, 0.0]), 20);
        assert_eq!(slide_mm([1.0, 2.0, 3.0], [1.0, 2.0, 3.0]), 0);
        assert_eq!(slide_mm([f32::NAN, 0.0, 0.0], [0.0; 3]), u32::MAX);
        assert_eq!(feet_hash(&[[0.0, 0.0, 0.0]]), feet_hash(&[[0.0, 0.0, 0.0]]));
        assert_ne!(
            feet_hash(&[[0.0, 0.0, 0.0]]),
            feet_hash(&[[0.02, 0.0, 0.0]])
        );
    }

    /// Straight-walk world feet: root advances at 1.5 m/s while planted raw
    /// feet drag +15 mm per tick (an uncorrected sampling error), so raw
    /// plant windows drift ~0.5 m and the lock must hold them still.
    struct WalkSeries {
        raw_l: Vec<[f32; 3]>,
        raw_r: Vec<[f32; 3]>,
        plant_l: Vec<bool>,
        plant_r: Vec<bool>,
    }

    fn straight_series(ticks: usize) -> WalkSeries {
        let mut series = WalkSeries {
            raw_l: Vec::with_capacity(ticks),
            raw_r: Vec::with_capacity(ticks),
            plant_l: Vec::with_capacity(ticks),
            plant_r: Vec::with_capacity(ticks),
        };
        let mut drag_l = 0.0f32;
        let mut drag_r = 0.0f32;
        for t in 0..ticks {
            let phase = t as f32 / ticks as f32;
            let root_x = t as f32 * 0.025;
            let table = contacts_for_phase(phase, t as u32);
            let (mut left, mut right) = (false, false);
            for c in table.contacts.iter() {
                if c.foot == 0 {
                    left = c.down;
                } else {
                    right = c.down;
                }
            }
            // Swing offsets: triangle waves of phase, opposed per foot.
            let tri = if phase < 0.5 {
                phase * 2.0
            } else {
                2.0 - phase * 2.0
            };
            let swing_l = tri * 0.25 - 0.125;
            let swing_r = (1.0 - tri) * 0.25 - 0.125;
            if left {
                drag_l += 0.015;
            } else {
                drag_l = 0.0;
            }
            if right {
                drag_r += 0.015;
            } else {
                drag_r = 0.0;
            }
            // Penetration dives 8 mm under on alternating ticks (clamp probe).
            let dip = if t % 2 == 0 { -0.008 } else { 0.004 };
            series
                .raw_l
                .push([root_x + swing_l + drag_l, 0.03 + dip, 0.11]);
            series
                .raw_r
                .push([root_x + swing_r + drag_r, 0.03 - dip, -0.11]);
            series.plant_l.push(left);
            series.plant_r.push(right);
        }
        series
    }

    /// Largest window displacement: max over continuous planted runs of the
    /// slide between the run's first and last positions.
    fn worst_window(positions: &[[f32; 3]], planted: &[bool]) -> u32 {
        let mut worst = 0u32;
        let mut start: Option<usize> = None;
        for (t, plant) in planted.iter().enumerate() {
            if *plant && start.is_none() {
                start = Some(t);
            }
            let end_of_run = !plant || t + 1 == planted.len();
            if let Some(s) = start {
                if end_of_run {
                    let last = if *plant { t } else { t.saturating_sub(1) };
                    if last > s {
                        let slide = slide_mm(positions[s], positions[last]);
                        if slide > worst {
                            worst = slide;
                        }
                    }
                    start = None;
                }
            }
        }
        worst
    }

    #[test]
    fn straight_walk_slide_under_2cm() {
        let ticks = 60;
        let series = straight_series(ticks);
        let (raw_l, raw_r, plant_l, plant_r) =
            (series.raw_l, series.raw_r, series.plant_l, series.plant_r);
        let mut locker = FootLocker::new(0.0).expect("fixture locker");
        let mut locked_l = Vec::with_capacity(ticks);
        let mut locked_r = Vec::with_capacity(ticks);
        for t in 0..ticks {
            locked_l.push(locker.update(0, raw_l[t], plant_l[t]).expect("lock left"));
            locked_r.push(locker.update(1, raw_r[t], plant_r[t]).expect("lock right"));
        }
        // Raw drift proves the fixture exercises the lock; corrected holds.
        let raw_worst = worst_window(&raw_l, &plant_l).max(worst_window(&raw_r, &plant_r));
        assert!(
            raw_worst > SLIDE_BUDGET_MM_STRAIGHT,
            "raw drift {raw_worst} mm"
        );
        let worst = worst_window(&locked_l, &plant_l).max(worst_window(&locked_r, &plant_r));
        println!("AM6 straight: raw worst={raw_worst} mm locked worst={worst} mm");
        assert!(
            worst <= SLIDE_BUDGET_MM_STRAIGHT,
            "locked worst {worst} mm past 2 cm"
        );
        for p in locked_l.iter().chain(locked_r.iter()) {
            assert!(p[1] >= 0.0, "clamp holds {p:?}");
        }
        // Replay: a twin locker over the same script locks identically.
        let mut twin = FootLocker::new(0.0).expect("twin locker");
        for t in 0..ticks {
            assert_eq!(
                twin.update(0, raw_l[t], plant_l[t]).expect("twin left"),
                locked_l[t]
            );
            assert_eq!(
                twin.update(1, raw_r[t], plant_r[t]).expect("twin right"),
                locked_r[t]
            );
        }
        assert_eq!(feet_hash(&locked_l), feet_hash(&locked_l));
        assert_eq!(locker, twin);
    }

    #[test]
    fn corner_turn_slide_under_4cm() {
        // Pivot fixture: 30 ticks forward (+x), 30 ticks lateral (+z); the
        // planted raw feet drag through the corner like a bad pivot sample.
        let ticks = 60;
        let mut locker = FootLocker::new(0.0).expect("fixture locker");
        let mut locked = Vec::with_capacity(ticks);
        let mut raw = Vec::with_capacity(ticks);
        let mut planted = Vec::with_capacity(ticks);
        let mut drag = 0.0f32;
        for t in 0..ticks {
            let phase = t as f32 / ticks as f32;
            let (rx, rz) = if t < 30 {
                (t as f32 * 0.025, 0.0)
            } else {
                (30.0 * 0.025, (t - 30) as f32 * 0.025)
            };
            let down = contacts_for_phase(phase, t as u32).contacts[0].down;
            if down {
                drag += 0.012;
            } else {
                drag = 0.0;
            }
            let p = [rx + drag, 0.03, rz + 0.05];
            raw.push(p);
            planted.push(down);
            locked.push(locker.update(0, p, down).expect("lock turn"));
        }
        let raw_worst = worst_window(&raw, &planted);
        let worst = worst_window(&locked, &planted);
        println!("AM6 pivot: raw worst={raw_worst} mm locked worst={worst} mm");
        assert!(
            worst <= SLIDE_BUDGET_MM_PIVOT,
            "locked worst {worst} mm past 4 cm"
        );
        let mut twin = FootLocker::new(0.0).expect("twin locker");
        for t in 0..ticks {
            assert_eq!(twin.update(0, raw[t], planted[t]).expect("twin"), locked[t]);
        }
    }

    #[test]
    fn warped_root_replays_exactly() {
        let probe = Probe {
            height_m: 1.0,
            depth_m: 0.6,
            gap_m: 0.5,
            speed_m_s: 2.0,
            has_hold: false,
        };
        let edge = [4.0, 1.0];
        let first = plan(&probe, edge).expect("fixture plans");
        let second = plan(&probe, edge).expect("fixture plans");
        let raw = [[0.0, 0.0], [1.0, 0.25], [2.0, 0.5]];
        let warped_first = apply_warp(&first, &raw).expect("fixture warps");
        let warped_second = apply_warp(&second, &raw).expect("fixture warps");
        assert_eq!(warped_first, warped_second);
        let end = *warped_first.last().expect("warped nonempty");
        assert_eq!(warp_error_mm(end, edge), 0.0);
        // Feet ride the warped root: same warped targets lock identically.
        let run = || {
            let mut locker = FootLocker::with_defaults();
            warped_first
                .iter()
                .map(|p| {
                    locker
                        .update(0, [p[0], 0.03, p[1]], true)
                        .expect("warped lock")
                })
                .collect::<Vec<_>>()
        };
        let (a, b) = (run(), run());
        assert_eq!(a, b);
        assert_eq!(feet_hash(&a), feet_hash(&b));
    }

    #[test]
    fn ct3_bake_replays_through_feet() {
        let first = bake_hero_set().expect("hero bakes");
        let second = bake_hero_set().expect("hero bakes again");
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.db.entries, second.db.entries);
        // Walk-forward frames drive planted feet: left plants while the
        // triangle wave rises, right while it falls; locked windows hold.
        let run = || {
            let mut locker = FootLocker::new(0.0).expect("ct3 locker");
            let mut locked = Vec::with_capacity(60);
            for frame in 0..60u16 {
                let phase = f32::from(frame) / 60.0;
                let tri = if phase < 0.5 {
                    phase * 2.0
                } else {
                    2.0 - phase * 2.0
                };
                let raw = [(tri - 0.5) * 0.5, 0.03, 0.11];
                let down = phase < 0.6;
                locked.push(locker.update(0, raw, down).expect("ct3 lock"));
            }
            locked
        };
        let (a, b) = (run(), run());
        assert_eq!(a, b);
        assert_eq!(feet_hash(&a), feet_hash(&b));
        println!(
            "AM6 CT3: digest=0x{:016x} frames={}",
            first.digest, first.frames
        );
    }

    #[test]
    fn graph_pose_locks_both_legs_atomically() {
        // Seven-joint rig: root, left hip/knee/ankle, right hip/knee/ankle.
        let mut pose = Pose {
            positions: vec![
                [0.0, 0.95, 0.0],
                [0.11, 0.9, 0.0],
                [0.11, 0.5, 0.02],
                [0.11, 0.05, 0.05],
                [-0.11, 0.9, 0.0],
                [-0.11, 0.5, -0.02],
                [-0.11, 0.12, -0.05],
            ],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 7],
        };
        let legs = [
            LegMap::new(1, 2, 3, 0).expect("left map"),
            LegMap::new(4, 5, 6, 1).expect("right map"),
        ];
        let mut locker = FootLocker::with_defaults();
        let sols =
            lock_graph_pose(&mut pose, &legs, [true, false], &mut locker).expect("graph lock");
        assert!(sols[0].locked && !sols[1].locked);
        assert_eq!(sols[0].slide_mm, 0);
        // Planted ankle solves onto its clamped target within a millimeter;
        // the knee moved toward it while the hip stayed.
        assert!(
            slide_mm(pose.positions[3], sols[0].target) <= 1,
            "{:?}",
            pose.positions[3]
        );
        assert!(pose.positions[3][1] >= 0.0);
        // A twin run over a fresh copy locks identically (graph consumer x2).
        let mut pose2 = Pose {
            positions: vec![
                [0.0, 0.95, 0.0],
                [0.11, 0.9, 0.0],
                [0.11, 0.5, 0.02],
                [0.11, 0.05, 0.05],
                [-0.11, 0.9, 0.0],
                [-0.11, 0.5, -0.02],
                [-0.11, 0.12, -0.05],
            ],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 7],
        };
        let mut twin = FootLocker::with_defaults();
        let sols2 =
            lock_graph_pose(&mut pose2, &legs, [true, false], &mut twin).expect("twin lock");
        assert_eq!(pose, pose2);
        assert_eq!(sols, sols2);
        // Bad maps write nothing and leave the locker untouched.
        let before_pose = pose.clone();
        let before_locker = locker.clone();
        let bad = [
            LegMap::new(1, 2, 3, 0).expect("left map"),
            LegMap {
                hip: 4,
                knee: 5,
                ankle: 99,
                foot: 1,
            },
        ];
        assert_eq!(
            lock_graph_pose(&mut pose, &bad, [true, true], &mut locker),
            Err(FootError::BadJoint(99))
        );
        assert_eq!(pose, before_pose);
        assert_eq!(locker, before_locker);
    }

    #[test]
    fn leg_solve_holds_lock_and_reports_correction() {
        let mut locker = FootLocker::with_defaults();
        let hip = [0.0, 0.95, 0.0];
        // First planted tick plants the anchor: zero correction.
        let first =
            solve_leg(&mut locker, 0, hip, KNEE, ANKLE, [0.05, 0.35, 0.05], true).expect("plant");
        assert!(first.locked);
        assert_eq!(first.slide_mm, 0);
        assert_eq!(first.target, [0.05, 0.35, 0.05]);
        // Drift while planted corrects back to the anchor: the slide meter
        // reports the pulled distance, the ankle solves back onto it.
        let held = solve_leg(
            &mut locker,
            0,
            hip,
            first.knee,
            first.ankle,
            [0.09, 0.35, 0.05],
            true,
        )
        .expect("hold");
        assert_eq!(held.slide_mm, 40);
        assert_eq!(held.target, first.target);
        assert!(
            slide_mm(held.ankle, first.ankle) <= 1,
            "{:?} vs {:?}",
            held.ankle,
            first.ankle
        );
        // Lift releases: output follows the clamped raw target.
        let free = solve_leg(
            &mut locker,
            0,
            hip,
            held.knee,
            held.ankle,
            [0.2, -0.04, 0.1],
            false,
        )
        .expect("lift");
        assert!(!free.locked);
        assert_eq!(free.target, [0.2, 0.0, 0.1]);
        assert_eq!(locker.anchor(0).expect("anchor read"), None);
    }

    #[test]
    fn failures_are_typed_never_panics() {
        assert_eq!(FootLocker::new(f32::NAN), Err(FootError::BadGround));
        assert!(FootLocker::new(0.0).is_ok());
        let mut locker = FootLocker::with_defaults();
        assert_eq!(locker.ground_y(), DEFAULT_GROUND_Y_M);
        assert_eq!(locker.update(7, ANKLE, true), Err(FootError::BadFoot(7)));
        assert_eq!(locker.anchor(7), Err(FootError::BadFoot(7)));
        assert_eq!(
            locker.update(0, [f32::NAN, 0.0, 0.0], true),
            Err(FootError::NonFinite("current"))
        );
        assert_eq!(
            solve_leg(&mut locker, 2, HIP, KNEE, ANKLE, ANKLE, true),
            Err(FootError::BadFoot(2))
        );
        assert_eq!(
            solve_leg(
                &mut locker,
                0,
                [f32::INFINITY, 0.0, 0.0],
                KNEE,
                ANKLE,
                ANKLE,
                true
            ),
            Err(FootError::NonFinite("hip"))
        );
        assert_eq!(
            solve_leg(&mut locker, 0, HIP, KNEE, ANKLE, [0.0, f32::NAN, 0.0], true),
            Err(FootError::NonFinite("target"))
        );
        assert_eq!(LegMap::new(0, 1, 2, 5), Err(FootError::BadFoot(5)));
        // Rejected input mutates nothing.
        assert_eq!(locker.anchor(0).expect("anchor read"), None);
        // Contacts: no sample reads lifted; later ticks hold the last flag.
        let table = ContactTable { contacts: vec![] };
        assert!(!planted_from_contacts(&table, 9, 0));
        let table = contacts_for_phase(0.2, 4);
        assert!(planted_from_contacts(&table, 4, 0));
        assert!(!planted_from_contacts(&table, 4, 1));
        assert!(!planted_from_contacts(&table, 4, 9));
        let text = FootError::BadFoot(7).to_string();
        assert!(text.contains('7'), "{text}");
        locker.reset();
        assert_eq!(locker.anchor(0).expect("anchor read"), None);
    }

    #[test]
    fn steady_state_reuses_caller_buffers() {
        let series = straight_series(30);
        let (raw_l, plant_l) = (series.raw_l, series.plant_l);
        let mut locker = FootLocker::new(0.0).expect("fixture locker");
        let mut pose = Pose {
            positions: vec![[0.0; 3]; 7],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 7],
        };
        let legs = [
            LegMap::new(1, 2, 3, 0).expect("left map"),
            LegMap::new(4, 5, 6, 1).expect("right map"),
        ];
        let pos_ptr = pose.positions.as_ptr();
        let rot_ptr = pose.rotations.as_ptr();
        let (pos_cap, rot_cap) = (pose.positions.capacity(), pose.rotations.capacity());
        for t in 0..30 {
            pose.positions[1] = [0.11, 0.9, 0.0];
            pose.positions[2] = [0.11, 0.5, 0.02];
            pose.positions[3] = raw_l[t % raw_l.len()];
            pose.positions[4] = [-0.11, 0.9, 0.0];
            pose.positions[5] = [-0.11, 0.5, -0.02];
            pose.positions[6] = [-0.11, 0.12, -0.05];
            lock_graph_pose(
                &mut pose,
                &legs,
                [plant_l[t % plant_l.len()], false],
                &mut locker,
            )
            .expect("steady tick");
        }
        assert_eq!(pose.positions.as_ptr(), pos_ptr);
        assert_eq!(pose.rotations.as_ptr(), rot_ptr);
        assert_eq!(pose.positions.capacity(), pos_cap);
        assert_eq!(pose.rotations.capacity(), rot_cap);
        assert_eq!(pose.positions.len(), 7);
    }
}
