//! FPS view typed diagnostic events: pose, hits, frame splits.
//!
//! Consumer: the endpoint `check_profile` pack reads `FpsFrameSplit`
//! (`from_stages` plus `sums_within`) for its profile verdict. Data owner:
//! [`FpsView`] owns three bounded inline rings; the caller owns the shared
//! [`RingBuffer`](super::RingBuffer) that [`record_view`] drains into.
//! Units: ticks (`u64`), millimetres (`i32`), yaw deci-degrees (`i16`),
//! microseconds (`u32`). Determinism: insertion order kept, oldest-first
//! iteration, saturating counters, no clock; same inputs give same details.
//! Error type: [`FpsError`]; invalid input fails, never panics.
//!
//! Diagnostics only: events never enter any simulation hash. Pushes store
//! inline structs with zero heap bytes; only `detail`/`to_event` format on
//! export, on the cold path.

use super::{Event, EventCode, RingBuffer};
use std::fmt;

/// Poses retained: 120 frames of a 2-fighter bout, the proof trace shape.
pub const MAX_FPS_POSES: usize = 240;

/// Hits retained: flood guard for one view window.
pub const MAX_FPS_HITS: usize = 64;

/// Frame splits retained: matches the 120-frame rerun ring.
pub const MAX_FPS_SPLITS: usize = 120;

/// Coordinate bound in millimetres (about 1 km each way).
pub const FPS_COORD_BOUND_MM: i32 = 1_000_000;

/// Yaw bound in deci-degrees (one full turn each way).
pub const FPS_YAW_BOUND_DDEG: i16 = 3600;

/// Split bound in microseconds (10 s; catches stalls without overflow).
pub const FPS_SPLIT_BOUND_US: u32 = 10_000_000;

/// Largest fighter slot, mirroring the input match slots.
pub const FPS_MAX_FIGHTER: u8 = 15;

/// Typed failure for FPS view construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpsError {
    /// Fighter slot above [`FPS_MAX_FIGHTER`].
    BadBody(u8),
    /// Coordinate outside [`FPS_COORD_BOUND_MM`].
    BadCoord(i32),
    /// Yaw outside [`FPS_YAW_BOUND_DDEG`].
    BadYaw(i16),
    /// Attacker slot above [`FPS_MAX_FIGHTER`].
    BadAttacker(u8),
    /// Target slot above [`FPS_MAX_FIGHTER`].
    BadTarget(u8),
    /// Attacker equals target.
    SelfHit(u8),
    /// Split field above [`FPS_SPLIT_BOUND_US`].
    BadSplit(u32),
}

impl fmt::Display for FpsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadBody(slot) => write!(f, "fps fighter out of range: {slot}"),
            Self::BadCoord(value) => write!(f, "fps coordinate out of range: {value}"),
            Self::BadYaw(yaw) => write!(f, "fps yaw out of range: {yaw}"),
            Self::BadAttacker(slot) => write!(f, "fps attacker out of range: {slot}"),
            Self::BadTarget(slot) => write!(f, "fps target out of range: {slot}"),
            Self::SelfHit(slot) => write!(f, "fps self hit: {slot}"),
            Self::BadSplit(value) => write!(f, "fps split out of range: {value}"),
        }
    }
}

impl std::error::Error for FpsError {}

/// One fighter pose in integer units for the FPS view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FpsPose {
    /// Tick the pose belongs to.
    pub tick: u64,
    /// Fighter slot, `0..=FPS_MAX_FIGHTER`.
    pub fighter: u8,
    /// Feet x in millimetres.
    pub x_mm: i32,
    /// Feet height in millimetres.
    pub y_mm: i32,
    /// Feet z in millimetres.
    pub z_mm: i32,
    /// Facing yaw in deci-degrees.
    pub yaw_ddeg: i16,
}

impl FpsPose {
    /// Validate raw fields; out-of-range values fail with [`FpsError`].
    ///
    /// # Errors
    ///
    /// Returns [`FpsError`] when a slot, coordinate or yaw is out of range.
    pub fn try_new(
        tick: u64,
        fighter: u8,
        x_mm: i32,
        y_mm: i32,
        z_mm: i32,
        yaw_ddeg: i16,
    ) -> Result<Self, FpsError> {
        if fighter > FPS_MAX_FIGHTER {
            return Err(FpsError::BadBody(fighter));
        }
        for value in [x_mm, y_mm, z_mm] {
            if !(-FPS_COORD_BOUND_MM..=FPS_COORD_BOUND_MM).contains(&value) {
                return Err(FpsError::BadCoord(value));
            }
        }
        if !(-FPS_YAW_BOUND_DDEG..=FPS_YAW_BOUND_DDEG).contains(&yaw_ddeg) {
            return Err(FpsError::BadYaw(yaw_ddeg));
        }
        Ok(Self {
            tick,
            fighter,
            x_mm,
            y_mm,
            z_mm,
            yaw_ddeg,
        })
    }

    /// Fixed-shape detail: `pose f=<n> x=<mm> y=<mm> z=<mm> yaw=<ddeg>`.
    #[must_use]
    pub fn detail(&self) -> String {
        format!(
            "pose f={} x={} y={} z={} yaw={}",
            self.fighter, self.x_mm, self.y_mm, self.z_mm, self.yaw_ddeg
        )
    }

    /// Convert to an observe event under `fps.pose`.
    #[must_use]
    pub fn to_event(&self) -> Event {
        Event::new(EventCode::FPS_POSE, self.tick, &self.detail())
    }
}

/// One adjudicated hit for the FPS view, mirroring the net verdict shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FpsHit {
    /// Authoritative tick at adjudication.
    pub server_tick: u64,
    /// Attacker-view tick as claimed.
    pub view_tick: u64,
    /// Attacker slot.
    pub attacker: u8,
    /// Target slot.
    pub target: u8,
    /// True when the lag window accepted the claim.
    pub accepted: bool,
}

impl FpsHit {
    /// Validate slots; tick distance is a verdict, never an error.
    ///
    /// # Errors
    ///
    /// Returns [`FpsError`] for bad slots or a self hit.
    pub fn try_new(
        server_tick: u64,
        view_tick: u64,
        attacker: u8,
        target: u8,
        accepted: bool,
    ) -> Result<Self, FpsError> {
        if attacker > FPS_MAX_FIGHTER {
            return Err(FpsError::BadAttacker(attacker));
        }
        if target > FPS_MAX_FIGHTER {
            return Err(FpsError::BadTarget(target));
        }
        if attacker == target {
            return Err(FpsError::SelfHit(attacker));
        }
        Ok(Self {
            server_tick,
            view_tick,
            attacker,
            target,
            accepted,
        })
    }

    /// Tick the event is filed under (authority time).
    #[must_use]
    pub fn tick(&self) -> u64 {
        self.server_tick
    }

    /// Fixed-shape detail: `hit a=<n> t=<n> server=<n> view=<n> accepted=<0|1>`.
    #[must_use]
    pub fn detail(&self) -> String {
        format!(
            "hit a={} t={} server={} view={} accepted={}",
            self.attacker,
            self.target,
            self.server_tick,
            self.view_tick,
            u8::from(self.accepted)
        )
    }

    /// Convert to an observe event under `fps.hit`.
    #[must_use]
    pub fn to_event(&self) -> Event {
        Event::new(EventCode::FPS_HIT, self.server_tick, &self.detail())
    }
}

/// One frame cost split for the FPS view, mirroring the arena receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FpsFrameSplit {
    /// Tick (or frame index) the split belongs to.
    pub tick: u64,
    /// Wall microseconds stepping fixed ticks.
    pub sim_us: u32,
    /// Wall microseconds presenting.
    pub present_us: u32,
    /// Wall microseconds of `present_us` spent on fighters.
    pub fighters_us: u32,
}

impl FpsFrameSplit {
    /// Per-stage costs in frame order: sim, present, fighters. Units: us.
    #[must_use]
    pub fn stages(&self) -> [u32; 3] {
        [self.sim_us, self.present_us, self.fighters_us]
    }

    /// Build from a per-stage list. Errors: [`FpsError::BadSplit`] above bound.
    pub fn from_stages(tick: u64, stages: [u32; 3]) -> Result<Self, FpsError> {
        for value in stages {
            if value > FPS_SPLIT_BOUND_US {
                return Err(FpsError::BadSplit(value));
            }
        }
        Ok(Self {
            tick,
            sim_us: stages[0],
            present_us: stages[1],
            fighters_us: stages[2],
        })
    }

    /// True when stages sum to `whole_us` within 5%. Integer math only.
    #[must_use]
    pub fn sums_within(&self, whole_us: u32) -> bool {
        let sum = u64::from(self.total_us());
        let whole = u64::from(whole_us);
        if whole == 0 {
            return sum == 0;
        }
        let diff = sum.abs_diff(whole);
        diff.saturating_mul(100) <= whole.saturating_mul(5)
    }

    /// Validate bounds; each field must fit [`FPS_SPLIT_BOUND_US`].
    ///
    /// # Errors
    ///
    /// Returns [`FpsError::BadSplit`] when any field is above the bound.
    pub fn try_new(
        tick: u64,
        sim_us: u32,
        present_us: u32,
        fighters_us: u32,
    ) -> Result<Self, FpsError> {
        Self::from_stages(tick, [sim_us, present_us, fighters_us])
    }

    /// Whole-frame cost: the three splits summed, saturating.
    #[must_use]
    pub fn total_us(&self) -> u32 {
        let s = self.stages();
        s[0].saturating_add(s[1]).saturating_add(s[2])
    }

    /// True when the whole-frame cost exceeds `budget_us`.
    #[must_use]
    pub fn over_budget(&self, budget_us: u32) -> bool {
        self.total_us() > budget_us
    }

    /// Fixed-shape detail: `split tick=<n> sim=<us> present=<us> fighters=<us>`.
    #[must_use]
    pub fn detail(&self) -> String {
        format!(
            "split tick={} sim={} present={} fighters={}",
            self.tick, self.sim_us, self.present_us, self.fighters_us
        )
    }

    /// Convert to an observe event under `fps.frame.split`.
    #[must_use]
    pub fn to_event(&self) -> Event {
        Event::new(EventCode::FPS_FRAME_SPLIT, self.tick, &self.detail())
    }
}

/// Bounded FPS view: inline rings for poses, hits and splits.
///
/// When full, pushing evicts the oldest row and counts the drop; rows are
/// never dropped silently. Backing storage is inline, so a push performs no
/// heap allocation. Total inline size stays under 10 KB.
#[derive(Debug, Clone)]
pub struct FpsView {
    poses: [FpsPose; MAX_FPS_POSES],
    pose_start: usize,
    pose_len: usize,
    pose_overflow: u64,
    hits: [FpsHit; MAX_FPS_HITS],
    hit_start: usize,
    hit_len: usize,
    hit_overflow: u64,
    splits: [FpsFrameSplit; MAX_FPS_SPLITS],
    split_start: usize,
    split_len: usize,
    split_overflow: u64,
}

impl FpsView {
    /// Empty view holding zero rows in each ring.
    #[must_use]
    pub fn new() -> Self {
        Self {
            poses: [FpsPose {
                tick: 0,
                fighter: 0,
                x_mm: 0,
                y_mm: 0,
                z_mm: 0,
                yaw_ddeg: 0,
            }; MAX_FPS_POSES],
            pose_start: 0,
            pose_len: 0,
            pose_overflow: 0,
            hits: [FpsHit {
                server_tick: 0,
                view_tick: 0,
                attacker: 0,
                target: 1,
                accepted: false,
            }; MAX_FPS_HITS],
            hit_start: 0,
            hit_len: 0,
            hit_overflow: 0,
            splits: [FpsFrameSplit {
                tick: 0,
                sim_us: 0,
                present_us: 0,
                fighters_us: 0,
            }; MAX_FPS_SPLITS],
            split_start: 0,
            split_len: 0,
            split_overflow: 0,
        }
    }

    /// Poses currently retained.
    #[must_use]
    pub fn pose_len(&self) -> usize {
        self.pose_len
    }

    /// Hits currently retained.
    #[must_use]
    pub fn hit_len(&self) -> usize {
        self.hit_len
    }

    /// Splits currently retained.
    #[must_use]
    pub fn split_len(&self) -> usize {
        self.split_len
    }

    /// Poses dropped because the ring was full. Monotonic.
    #[must_use]
    pub fn pose_overflow(&self) -> u64 {
        self.pose_overflow
    }

    /// Hits dropped because the ring was full. Monotonic.
    #[must_use]
    pub fn hit_overflow(&self) -> u64 {
        self.hit_overflow
    }

    /// Splits dropped because the ring was full. Monotonic.
    #[must_use]
    pub fn split_overflow(&self) -> u64 {
        self.split_overflow
    }

    /// Record one pose, evicting the oldest when full. Never allocates.
    pub fn push_pose(&mut self, pose: FpsPose) {
        if self.pose_len < MAX_FPS_POSES {
            let pos = (self.pose_start + self.pose_len) % MAX_FPS_POSES;
            self.poses[pos] = pose;
            self.pose_len += 1;
        } else {
            self.poses[self.pose_start] = pose;
            self.pose_start = (self.pose_start + 1) % MAX_FPS_POSES;
            self.pose_overflow = self.pose_overflow.saturating_add(1);
        }
    }

    /// Record one hit, evicting the oldest when full. Never allocates.
    pub fn push_hit(&mut self, hit: FpsHit) {
        if self.hit_len < MAX_FPS_HITS {
            let pos = (self.hit_start + self.hit_len) % MAX_FPS_HITS;
            self.hits[pos] = hit;
            self.hit_len += 1;
        } else {
            self.hits[self.hit_start] = hit;
            self.hit_start = (self.hit_start + 1) % MAX_FPS_HITS;
            self.hit_overflow = self.hit_overflow.saturating_add(1);
        }
    }

    /// Record one split, evicting the oldest when full. Never allocates.
    pub fn push_split(&mut self, split: FpsFrameSplit) {
        if self.split_len < MAX_FPS_SPLITS {
            let pos = (self.split_start + self.split_len) % MAX_FPS_SPLITS;
            self.splits[pos] = split;
            self.split_len += 1;
        } else {
            self.splits[self.split_start] = split;
            self.split_start = (self.split_start + 1) % MAX_FPS_SPLITS;
            self.split_overflow = self.split_overflow.saturating_add(1);
        }
    }

    /// Oldest-first iterator over retained poses.
    pub fn iter_poses(&self) -> impl DoubleEndedIterator<Item = &FpsPose> + '_ {
        (0..self.pose_len).map(move |index| {
            let pos = (self.pose_start + index) % MAX_FPS_POSES;
            &self.poses[pos]
        })
    }

    /// Oldest-first iterator over retained hits.
    pub fn iter_hits(&self) -> impl DoubleEndedIterator<Item = &FpsHit> + '_ {
        (0..self.hit_len).map(move |index| {
            let pos = (self.hit_start + index) % MAX_FPS_HITS;
            &self.hits[pos]
        })
    }

    /// Oldest-first iterator over retained splits.
    pub fn iter_splits(&self) -> impl DoubleEndedIterator<Item = &FpsFrameSplit> + '_ {
        (0..self.split_len).map(move |index| {
            let pos = (self.split_start + index) % MAX_FPS_SPLITS;
            &self.splits[pos]
        })
    }
}

impl Default for FpsView {
    /// Empty view; same as [`FpsView::new`].
    fn default() -> Self {
        Self::new()
    }
}

/// Drain one view into the shared ring oldest-first: poses, hits, splits.
///
/// The caller keeps owning the view; this only exports typed rows as
/// `fps.pose` / `fps.hit` / `fps.frame.split` events. Read by the endpoint
/// `check_profile` pack through `FpsFrameSplit::from_stages`.
pub fn record_view(view: &FpsView, ring: &mut RingBuffer) {
    for pose in view.iter_poses() {
        ring.push_pose(*pose);
    }
    for hit in view.iter_hits() {
        ring.push_hit(*hit);
    }
    for split in view.iter_splits() {
        ring.push_split(*split);
    }
}

#[cfg(test)]
mod tests {
    use super::super::MAX_MESSAGE_LEN;
    use super::*;

    fn pose(tick: u64, fighter: u8) -> FpsPose {
        FpsPose::try_new(tick, fighter, 1200, 0, -400, 900).unwrap_or(FpsPose {
            tick,
            fighter,
            x_mm: 0,
            y_mm: 0,
            z_mm: 0,
            yaw_ddeg: 0,
        })
    }

    #[test]
    fn pose_rejects_out_of_range_fields() {
        assert_eq!(
            FpsPose::try_new(0, 16, 0, 0, 0, 0),
            Err(FpsError::BadBody(16))
        );
        assert_eq!(
            FpsPose::try_new(0, 0, FPS_COORD_BOUND_MM + 1, 0, 0, 0),
            Err(FpsError::BadCoord(FPS_COORD_BOUND_MM + 1))
        );
        assert_eq!(
            FpsPose::try_new(0, 0, 0, 0, 0, FPS_YAW_BOUND_DDEG + 1),
            Err(FpsError::BadYaw(FPS_YAW_BOUND_DDEG + 1))
        );
        let err = FpsError::BadBody(99);
        assert_eq!(err.to_string(), "fps fighter out of range: 99");
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn hit_rejects_bad_slots_and_self_hit() {
        assert_eq!(
            FpsHit::try_new(10, 9, 16, 1, true),
            Err(FpsError::BadAttacker(16))
        );
        assert_eq!(
            FpsHit::try_new(10, 9, 0, 16, false),
            Err(FpsError::BadTarget(16))
        );
        assert_eq!(
            FpsHit::try_new(10, 9, 2, 2, true),
            Err(FpsError::SelfHit(2))
        );
        // Tick distance is a verdict, never an error: ahead views record.
        let ahead = FpsHit::try_new(10, 12, 0, 1, false).expect("ticks are verdicts");
        assert!(!ahead.accepted);
        assert_eq!(ahead.tick(), 10);
    }

    #[test]
    fn split_bound_total_and_budget() {
        assert_eq!(
            FpsFrameSplit::try_new(0, FPS_SPLIT_BOUND_US + 1, 0, 0),
            Err(FpsError::BadSplit(FPS_SPLIT_BOUND_US + 1))
        );
        let split = FpsFrameSplit::try_new(7, 120, 2300, 400).expect("in bound");
        assert_eq!(split.total_us(), 2820);
        assert!(split.over_budget(1000));
        assert!(!split.over_budget(2820));
        assert!(!split.over_budget(5000));
        // Saturation never wraps past u32::MAX.
        let wide = FpsFrameSplit {
            tick: 0,
            sim_us: u32::MAX,
            present_us: u32::MAX,
            fighters_us: u32::MAX,
        };
        assert_eq!(wide.total_us(), u32::MAX);
    }

    #[test]
    fn typed_details_stay_bounded_with_stable_codes() {
        let pose = FpsPose::try_new(3, 1, -1200, 50, 400, -900).expect("valid pose");
        let hit = FpsHit::try_new(100, 98, 0, 1, true).expect("valid hit");
        let split = FpsFrameSplit::try_new(100, 120, 2300, 400).expect("valid split");
        assert_eq!(pose.to_event().code, EventCode::FPS_POSE);
        assert_eq!(hit.to_event().code, EventCode::FPS_HIT);
        assert_eq!(split.to_event().code, EventCode::FPS_FRAME_SPLIT);
        assert_eq!(pose.to_event().tick, 3);
        assert_eq!(hit.to_event().tick, 100);
        assert_eq!(split.to_event().tick, 100);
        assert!(pose.detail().len() <= MAX_MESSAGE_LEN);
        assert!(hit.detail().len() <= MAX_MESSAGE_LEN);
        assert!(split.detail().len() <= MAX_MESSAGE_LEN);
        assert!(pose.detail().starts_with("pose f=1"));
        assert!(hit.detail().starts_with("hit a=0 t=1"));
        assert!(split.detail().starts_with("split tick=100"));
        // New codes round-trip through the stable registry.
        assert_eq!(EventCode::from_raw(16), Ok(EventCode::FPS_POSE));
        assert_eq!(EventCode::from_raw(17), Ok(EventCode::FPS_HIT));
        assert_eq!(EventCode::from_raw(18), Ok(EventCode::FPS_FRAME_SPLIT));
    }

    #[test]
    fn view_evicts_oldest_and_counts_overflow() {
        let mut view = FpsView::new();
        assert_eq!(view.pose_len(), 0);
        assert_eq!(view.hit_len(), 0);
        assert_eq!(view.split_len(), 0);
        for tick in 0..u64::try_from(MAX_FPS_POSES + 5).unwrap_or(u64::MAX) {
            view.push_pose(pose(tick, 0));
        }
        assert_eq!(view.pose_len(), MAX_FPS_POSES);
        assert_eq!(view.pose_overflow(), 5);
        let ticks: Vec<u64> = view.iter_poses().map(|row| row.tick).collect();
        assert_eq!(ticks[0], 5);
        assert_eq!(
            ticks[ticks.len() - 1],
            u64::try_from(MAX_FPS_POSES + 4).unwrap_or(0)
        );

        for tick in 0..u64::try_from(MAX_FPS_HITS + 2).unwrap_or(u64::MAX) {
            let hit = FpsHit::try_new(tick, tick, 0, 1, true).expect("slots stay valid");
            view.push_hit(hit);
        }
        assert_eq!(view.hit_len(), MAX_FPS_HITS);
        assert_eq!(view.hit_overflow(), 2);

        for tick in 0..u64::try_from(MAX_FPS_SPLITS + 1).unwrap_or(u64::MAX) {
            let tick_u64 = tick;
            let split = FpsFrameSplit::try_new(tick_u64, 10, 20, 30).expect("splits stay valid");
            view.push_split(split);
        }
        assert_eq!(view.split_len(), MAX_FPS_SPLITS);
        assert_eq!(view.split_overflow(), 1);
    }

    #[test]
    fn record_view_drains_oldest_first_into_ring() {
        let mut view = FpsView::new();
        view.push_pose(pose(1, 0));
        view.push_pose(pose(2, 1));
        view.push_hit(FpsHit::try_new(2, 1, 0, 1, true).expect("valid hit"));
        view.push_split(FpsFrameSplit::try_new(2, 10, 20, 30).expect("valid split"));
        let mut ring = RingBuffer::new(16);
        record_view(&view, &mut ring);
        assert_eq!(ring.len(), 4);
        assert_eq!(ring.query(Some(EventCode::FPS_POSE)).len(), 2);
        assert_eq!(ring.query(Some(EventCode::FPS_HIT)).len(), 1);
        assert_eq!(ring.query(Some(EventCode::FPS_FRAME_SPLIT)).len(), 1);
        let ticks: Vec<u64> = ring
            .query(Some(EventCode::FPS_POSE))
            .iter()
            .map(|event| event.tick)
            .collect();
        assert_eq!(ticks, vec![1, 2]);
    }

    #[test]
    fn same_view_twice_gives_same_details() {
        fn build() -> FpsView {
            let mut view = FpsView::new();
            view.push_pose(pose(9, 3));
            view.push_hit(FpsHit::try_new(9, 8, 0, 1, false).expect("valid hit"));
            view.push_split(FpsFrameSplit::try_new(9, 1, 2, 3).expect("valid split"));
            view
        }
        let left = build();
        let right = build();
        let left_details: Vec<String> = left
            .iter_poses()
            .map(FpsPose::detail)
            .chain(left.iter_hits().map(FpsHit::detail))
            .chain(left.iter_splits().map(FpsFrameSplit::detail))
            .collect();
        let right_details: Vec<String> = right
            .iter_poses()
            .map(FpsPose::detail)
            .chain(right.iter_hits().map(FpsHit::detail))
            .chain(right.iter_splits().map(FpsFrameSplit::detail))
            .collect();
        assert_eq!(left_details, right_details);
    }
}
