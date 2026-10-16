//! traversal: authoritative traversal chooser + edge warp.
//!
//! Consumer: arena locomotion traversal ranks vault/hurdle/mantle/climb/drop
//! from obstacle probes and warps root motion onto edges. Data owner:
//! `engine-anim`; this module never imports assets or arena. Units: meters,
//! ticks at 60 Hz (millimeters inside the authority path).
//!
//! Determinism: validated probes convert to integer millimeters once, the
//! chooser compares integers in a fixed order, and the warp is an exact
//! integer translation, so the same probe plus the same edge replays
//! byte-identically. Only `+ - * /` and [`f32::sqrt`] (error measure).
//! Errors are [`TraverseError`].
//!
//! Motion-DB approach ranking ([`motiondb`]) scores one committed plan's
//! approach clip over a bounded [`motiondb::DbSlice`]; vault/mantle/climb
//! approach params ([`action_params`]) ride over locomotion readings through
//! [`loco_gate`]. Caller (future): `crates/arena/src/spine_step.rs`
//! (`sys_motion`) ranks one approach clip per committed plan.

#[path = "motiondb.rs"]
pub mod motiondb;

use self::motiondb::{DbSlice, SliceError};
use super::locomotion::Locomotion;
use super::motion_match::{
    build_query, cost_units, default_weights, predict_linear, MatchError, MatchResult, MotionDb,
    PoseSample, Query, SearchReceipt,
};
use engine_core::Fnv64;

/// Millimeters per meter inside the authority path.
pub const MM_PER_M: f32 = 1000.0;
/// Warp acceptance: the warped root must land within 1 cm of the edge.
pub const WARP_TOL_MM: f32 = 10.0;
/// Widest clearable gap; wider probes refuse with [`TraverseError::GapTooWide`].
pub const MAX_GAP_MM: i32 = 2500;
/// Below this rise the actor steps through with no traversal move.
pub const STEP_MM: i32 = 200;
/// Tallest top cleared as a hurdle.
pub const HURDLE_MAX_MM: i32 = 750;
/// Tallest top cleared as a vault.
pub const VAULT_MAX_MM: i32 = 1250;
/// Tallest top cleared as a mantle.
pub const MANTLE_MAX_MM: i32 = 1700;
/// Tallest top cleared as a climb (needs a hold).
pub const CLIMB_MAX_MM: i32 = 2600;
/// Widest obstacle hurdled; wider low blocks are mantled onto.
pub const HURDLE_DEPTH_MM: i32 = 800;
/// Widest obstacle vaulted; wider chest blocks are mantled onto.
pub const VAULT_DEPTH_MM: i32 = 1200;
/// Slow approaches over hurdle-high rails are mantled, not hurdled.
pub const HURDLE_MIN_SPEED_MM_S: i32 = 1000;
/// Drops start past this depth below the feet.
pub const DROP_EDGE_MM: i32 = -300;
/// Deeper drops than this are refused, never silently clamped.
pub const DROP_MAX_MM: i32 = -2500;
/// Longest warpable root path; bounds per-plan allocation.
pub const MAX_PATH: usize = 512;
/// Probe height lane range (m).
pub const PROBE_HEIGHT_RANGE_M: f32 = 5.0;
/// Probe depth/gap lane range (m).
pub const PROBE_SPAN_RANGE_M: f32 = 5.0;
/// Fastest accepted approach speed (m/s); matches locomotion's wish cap.
pub const PROBE_SPEED_RANGE_M_S: f32 = 12.0;
/// Edge anchor range (m) per lane.
pub const EDGE_RANGE_M: f32 = 1000.0;

/// Traversal failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TraverseError {
    /// A probe, edge or path lane is out of bounds; the lane names it.
    BadProbe(&'static str),
    /// A climb-high top with no hold; the move is refused, never faked.
    NoHold,
    /// The gap exceeds what any move clears; millimeters carried.
    GapTooWide {
        /// Probe gap in millimeters.
        gap_mm: i32,
        /// Widest clearable gap in millimeters.
        max_mm: i32,
    },
}

impl core::fmt::Display for TraverseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TraverseError::BadProbe(lane) => write!(f, "traversal: bad probe ({lane})"),
            TraverseError::NoHold => write!(f, "traversal: climb needs a hold"),
            TraverseError::GapTooWide { gap_mm, max_mm } => {
                write!(f, "traversal: gap {gap_mm} mm past {max_mm} mm")
            }
        }
    }
}

impl std::error::Error for TraverseError {}

/// Traversal move the chooser commits to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TraverseAction {
    /// Flat or step ground: no traversal move.
    None,
    /// Low rail taken at speed.
    Hurdle,
    /// Waist-high block vaulted across.
    Vault,
    /// Chest-high ledge mantled onto.
    Mantle,
    /// High wall climbed with a hold.
    Climb,
    /// Ledge dropped from.
    Drop,
}

/// Obstacle probe from arena locomotion: obstacle top relative to the feet
/// (`height_m`, up positive), block thickness (`depth_m`), horizontal gap to
/// the far edge (`gap_m`), approach speed, and whether a climb hold exists.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Probe {
    /// Obstacle top in meters above the feet (drops go negative).
    pub height_m: f32,
    /// Block thickness in meters along travel.
    pub depth_m: f32,
    /// Horizontal gap in meters to the far edge.
    pub gap_m: f32,
    /// Approach speed in meters per second.
    pub speed_m_s: f32,
    /// True when a climb hold is available.
    pub has_hold: bool,
}

/// Probe in integer millimeters; the only form the chooser compares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProbeFix {
    /// Obstacle top in millimeters above the feet.
    pub height_mm: i32,
    /// Block thickness in millimeters.
    pub depth_mm: i32,
    /// Gap in millimeters.
    pub gap_mm: i32,
    /// Approach speed in millimeters per second.
    pub speed_mm_s: i32,
    /// True when a climb hold is available.
    pub has_hold: bool,
}

/// Fixed ticks per move at 60 Hz.
pub fn duration_ticks(action: TraverseAction) -> u32 {
    match action {
        TraverseAction::None => 0,
        TraverseAction::Hurdle => 20,
        TraverseAction::Vault => 30,
        TraverseAction::Mantle => 40,
        TraverseAction::Climb => 60,
        TraverseAction::Drop => 25,
    }
}

/// Approach-clip params per traversal move. Caller: [`plan`] commits
/// durations from here; [`rank_approach`] biases the approach query from the
/// locomotion lanes. Units: ticks at 60 Hz, millimeters per second,
/// millimeters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MoveParams {
    /// Fixed ticks at 60 Hz; always [`duration_ticks`] of the action.
    pub duration_ticks: u32,
    /// Advisory approach speed in mm/s for the approach-clip bias: below it
    /// the rank leans on continuity instead of hurrying the clip. Vault
    /// shares the hurdle run-up lane. Never a chooser gate; gates stay in
    /// the fixed chooser.
    pub min_approach_mm_s: i32,
    /// Widest block this move clears in mm; [`i32::MAX`] ends on top, so any
    /// depth reads as clearable.
    pub max_depth_mm: i32,
    /// True when the move refuses without a hold.
    pub needs_hold: bool,
}

/// Rich params for one move: durations ride [`duration_ticks`], depth lanes
/// ride the chooser bounds, holds ride the climb gate. Determinism: pure
/// table lookup, no floats.
pub fn action_params(action: TraverseAction) -> MoveParams {
    let ticks = duration_ticks(action);
    match action {
        TraverseAction::None => MoveParams {
            duration_ticks: ticks,
            min_approach_mm_s: 0,
            max_depth_mm: 0,
            needs_hold: false,
        },
        TraverseAction::Hurdle => MoveParams {
            duration_ticks: ticks,
            min_approach_mm_s: HURDLE_MIN_SPEED_MM_S,
            max_depth_mm: HURDLE_DEPTH_MM,
            needs_hold: false,
        },
        TraverseAction::Vault => MoveParams {
            duration_ticks: ticks,
            min_approach_mm_s: HURDLE_MIN_SPEED_MM_S,
            max_depth_mm: VAULT_DEPTH_MM,
            needs_hold: false,
        },
        TraverseAction::Mantle => MoveParams {
            duration_ticks: ticks,
            min_approach_mm_s: 0,
            max_depth_mm: i32::MAX,
            needs_hold: false,
        },
        TraverseAction::Climb => MoveParams {
            duration_ticks: ticks,
            min_approach_mm_s: 0,
            max_depth_mm: i32::MAX,
            needs_hold: true,
        },
        TraverseAction::Drop => MoveParams {
            duration_ticks: ticks,
            min_approach_mm_s: 0,
            max_depth_mm: 0,
            needs_hold: false,
        },
    }
}

/// Round meters to integer millimeters; callers validate finiteness first.
fn to_mm(value: f32) -> i32 {
    (value * MM_PER_M).round() as i32
}

/// Validate a probe and freeze it to integer millimeters.
pub fn fix_probe(probe: &Probe) -> Result<ProbeFix, TraverseError> {
    if !probe.height_m.is_finite() {
        return Err(TraverseError::BadProbe("height"));
    }
    if !probe.depth_m.is_finite() {
        return Err(TraverseError::BadProbe("depth"));
    }
    if !probe.gap_m.is_finite() {
        return Err(TraverseError::BadProbe("gap"));
    }
    if !probe.speed_m_s.is_finite() {
        return Err(TraverseError::BadProbe("speed"));
    }
    if probe.height_m < -PROBE_HEIGHT_RANGE_M || probe.height_m > PROBE_HEIGHT_RANGE_M {
        return Err(TraverseError::BadProbe("height"));
    }
    if !(0.0..=PROBE_SPAN_RANGE_M).contains(&probe.depth_m) {
        return Err(TraverseError::BadProbe("depth"));
    }
    if !(0.0..=PROBE_SPAN_RANGE_M).contains(&probe.gap_m) {
        return Err(TraverseError::BadProbe("gap"));
    }
    if !(0.0..=PROBE_SPEED_RANGE_M_S).contains(&probe.speed_m_s) {
        return Err(TraverseError::BadProbe("speed"));
    }
    Ok(ProbeFix {
        height_mm: to_mm(probe.height_m),
        depth_mm: to_mm(probe.depth_m),
        gap_mm: to_mm(probe.gap_m),
        speed_mm_s: to_mm(probe.speed_m_s),
        has_hold: probe.has_hold,
    })
}

/// Choose the traversal move for a fixed probe. Gap refusal comes first so
/// no move is ever picked past what any of them can clear.
fn choose_fix(fixed: &ProbeFix) -> Result<TraverseAction, TraverseError> {
    if fixed.gap_mm > MAX_GAP_MM {
        return Err(TraverseError::GapTooWide {
            gap_mm: fixed.gap_mm,
            max_mm: MAX_GAP_MM,
        });
    }
    let h = fixed.height_mm;
    if h <= DROP_EDGE_MM {
        if h < DROP_MAX_MM {
            return Err(TraverseError::BadProbe("drop"));
        }
        return Ok(TraverseAction::Drop);
    }
    if h < STEP_MM {
        return Ok(TraverseAction::None);
    }
    if h <= HURDLE_MAX_MM {
        if fixed.depth_mm <= HURDLE_DEPTH_MM && fixed.speed_mm_s >= HURDLE_MIN_SPEED_MM_S {
            return Ok(TraverseAction::Hurdle);
        }
        return Ok(TraverseAction::Mantle);
    }
    if h <= VAULT_MAX_MM {
        if fixed.depth_mm <= VAULT_DEPTH_MM {
            return Ok(TraverseAction::Vault);
        }
        return Ok(TraverseAction::Mantle);
    }
    if h <= MANTLE_MAX_MM {
        return Ok(TraverseAction::Mantle);
    }
    if h <= CLIMB_MAX_MM {
        if fixed.has_hold {
            return Ok(TraverseAction::Climb);
        }
        return Err(TraverseError::NoHold);
    }
    Err(TraverseError::BadProbe("height"))
}

/// Choose the traversal move for an obstacle probe.
pub fn choose(probe: &Probe) -> Result<TraverseAction, TraverseError> {
    choose_fix(&fix_probe(probe)?)
}

/// Committed traversal plan: the chosen move, its tick budget, the edge the
/// warped root must land on, and the fixed probe it was chosen from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TraversalPlan {
    /// Chosen move.
    pub action: TraverseAction,
    /// Tick budget at 60 Hz.
    pub duration_ticks: u32,
    /// Edge anchor in millimeters `[x, z]`.
    pub edge_mm: [i32; 2],
    /// Fixed probe this plan was chosen from.
    pub probe: ProbeFix,
}

/// Validate a planar edge anchor in meters.
fn fix_edge(edge_m: [f32; 2]) -> Result<[i32; 2], TraverseError> {
    if edge_m.iter().any(|v| !v.is_finite()) {
        return Err(TraverseError::BadProbe("edge"));
    }
    if edge_m
        .iter()
        .any(|v| *v < -EDGE_RANGE_M || *v > EDGE_RANGE_M)
    {
        return Err(TraverseError::BadProbe("edge"));
    }
    Ok([to_mm(edge_m[0]), to_mm(edge_m[1])])
}

/// Commit a plan for a probe plus the edge the warped root must land on.
pub fn plan(probe: &Probe, edge_m: [f32; 2]) -> Result<TraversalPlan, TraverseError> {
    let fixed = fix_probe(probe)?;
    let action = choose_fix(&fixed)?;
    Ok(TraversalPlan {
        duration_ticks: action_params(action).duration_ticks,
        action,
        edge_mm: fix_edge(edge_m)?,
        probe: fixed,
    })
}

/// Validate locomotion readings for an approach rank: `speed_m_s` carries
/// `Locomotion::speed`, `airborne` carries `Locomotion::is_airborne`.
/// Caller: [`rank_approach`]. Air moves refuse unless the plan drops: a
/// vault, mantle or climb approach clip is meaningless mid-air, while a drop
/// commits from the fall. Rejects non-finite, negative and over-limit speeds
/// without touching anything.
pub fn loco_gate(
    action: TraverseAction,
    speed_m_s: f32,
    airborne: bool,
) -> Result<(), TraverseError> {
    if !speed_m_s.is_finite() || speed_m_s < 0.0 || speed_m_s > PROBE_SPEED_RANGE_M_S {
        return Err(TraverseError::BadProbe("speed"));
    }
    if airborne && action != TraverseAction::Drop {
        return Err(TraverseError::BadProbe("airborne"));
    }
    Ok(())
}

/// Build the approach query for a committed move: the pose sample carries the
/// probe speed as hip velocity with a neutral stance, the trajectory is the
/// linear rollout, and the penalties bias holds per move (climbs hold plants,
/// drops travel light). Caller: [`rank_approach`]. Contact intent is planted
/// for ground moves, open for drops.
pub fn approach_query(
    probe: &Probe,
    action: TraverseAction,
    curr: u32,
) -> Result<Query, TraverseError> {
    fix_probe(probe)?;
    let contact = if action == TraverseAction::Drop { 0 } else { 3 };
    let sample = PoseSample {
        hip_vel: [0.0, probe.speed_m_s],
        foot_l: [0.11, 0.05, 0.0],
        foot_r: [-0.11, 0.05, 0.0],
        contact,
    };
    let traj = predict_linear([0.0, 0.0], sample.hip_vel)
        .map_err(|_| TraverseError::BadProbe("trajectory"))?;
    let contact_cost = match action {
        TraverseAction::Climb => cost_units(0.09),
        TraverseAction::Drop => cost_units(0.01),
        _ => cost_units(0.04),
    };
    build_query(
        &sample,
        &traj,
        curr,
        &default_weights(),
        cost_units(0.01),
        contact_cost,
    )
    .map_err(|err| match err {
        MatchError::BadQuery(lane) => TraverseError::BadProbe(lane),
        MatchError::BadDatabase(_)
        | MatchError::BadNetDims { .. }
        | MatchError::Net(_)
        | MatchError::EmptyDatabase => TraverseError::BadProbe("query"),
    })
}

/// Rank one approach clip for a committed move over `slice`: gates the
/// locomotion link (`loco` carries `Locomotion::speed` plus
/// `Locomotion::is_airborne`), shapes the query for `action`, and scans the
/// slice. Returns the winning clip frame plus the per-character cost receipt.
/// Caller (future): `crates/arena/src/spine_step.rs` (`sys_motion`) ranks one
/// approach clip per committed plan; that arena wiring is the stated
/// remainder. Slice and query failures map to [`TraverseError`].
pub fn rank_approach(
    db: &MotionDb,
    probe: &Probe,
    action: TraverseAction,
    loco: &Locomotion,
    slice: DbSlice,
    curr: u32,
) -> Result<(MatchResult, SearchReceipt), TraverseError> {
    loco_gate(action, loco.speed(), loco.is_airborne())?;
    let query = approach_query(probe, action, curr)?;
    motiondb::search_slice(db, &slice, &query).map_err(|err| match err {
        SliceError::EmptyDatabase => TraverseError::BadProbe("empty database"),
        SliceError::BadSlice(lane) => TraverseError::BadProbe(lane),
        SliceError::BadQuery(lane) => TraverseError::BadProbe(lane),
    })
}

/// FNV-1a64 over the plan lanes; identical inputs hash identically.
pub fn plan_hash(plan: &TraversalPlan) -> u64 {
    let mut h = Fnv64::new();
    let disc: u8 = match plan.action {
        TraverseAction::None => 0,
        TraverseAction::Hurdle => 1,
        TraverseAction::Vault => 2,
        TraverseAction::Mantle => 3,
        TraverseAction::Climb => 4,
        TraverseAction::Drop => 5,
    };
    h.write_u8(disc);
    h.write(&plan.duration_ticks.to_le_bytes());
    for lane in plan.edge_mm.iter() {
        h.write(&lane.to_le_bytes());
    }
    for lane in [
        plan.probe.height_mm,
        plan.probe.depth_mm,
        plan.probe.gap_mm,
        plan.probe.speed_mm_s,
    ] {
        h.write(&lane.to_le_bytes());
    }
    h.write_bool(plan.probe.has_hold);
    h.finish()
}

/// Warp a raw planar root path in meters onto the plan edge: an exact
/// integer translation (`warped = raw + (edge - raw_end)`), so the warped
/// end lands on the edge and the path shape is preserved.
pub fn apply_warp(plan: &TraversalPlan, raw: &[[f32; 2]]) -> Result<Vec<[f32; 2]>, TraverseError> {
    if raw.is_empty() {
        return Err(TraverseError::BadProbe("path empty"));
    }
    if raw.len() > MAX_PATH {
        return Err(TraverseError::BadProbe("path too long"));
    }
    if raw.iter().flatten().any(|v| !v.is_finite()) {
        return Err(TraverseError::BadProbe("path"));
    }
    let mut path_mm = Vec::with_capacity(raw.len());
    for point in raw {
        path_mm.push([to_mm(point[0]), to_mm(point[1])]);
    }
    let end = match path_mm.last() {
        Some(end) => *end,
        None => return Err(TraverseError::BadProbe("path empty")),
    };
    let offset = [
        plan.edge_mm[0].saturating_sub(end[0]),
        plan.edge_mm[1].saturating_sub(end[1]),
    ];
    Ok(path_mm
        .iter()
        .map(|point| {
            [
                point[0].saturating_add(offset[0]) as f32 / MM_PER_M,
                point[1].saturating_add(offset[1]) as f32 / MM_PER_M,
            ]
        })
        .collect())
}

/// Residual miss distance in millimeters between the warped root end and
/// the edge. The acceptance gate is `<= WARP_TOL_MM` (1 cm).
pub fn warp_error_mm(warped_end_m: [f32; 2], edge_m: [f32; 2]) -> f32 {
    let dx = (warped_end_m[0] - edge_m[0]) * MM_PER_M;
    let dy = (warped_end_m[1] - edge_m[1]) * MM_PER_M;
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(height: f32, depth: f32, gap: f32, speed: f32, hold: bool) -> Probe {
        Probe {
            height_m: height,
            depth_m: depth,
            gap_m: gap,
            speed_m_s: speed,
            has_hold: hold,
        }
    }

    #[test]
    fn fixtures_pick_expected_actions() {
        assert_eq!(
            choose(&probe(0.0, 0.0, 0.0, 1.5, false)),
            Ok(TraverseAction::None)
        );
        assert_eq!(
            choose(&probe(0.5, 0.3, 0.4, 3.0, false)),
            Ok(TraverseAction::Hurdle)
        );
        assert_eq!(
            choose(&probe(1.0, 0.6, 0.5, 2.0, false)),
            Ok(TraverseAction::Vault)
        );
        assert_eq!(
            choose(&probe(1.5, 2.0, 0.2, 1.0, false)),
            Ok(TraverseAction::Mantle)
        );
        assert_eq!(
            choose(&probe(2.0, 0.4, 0.3, 1.0, true)),
            Ok(TraverseAction::Climb)
        );
        assert_eq!(
            choose(&probe(-1.0, 0.5, 0.4, 1.5, false)),
            Ok(TraverseAction::Drop)
        );
    }

    #[test]
    fn boundary_heights_pick_the_lower_move() {
        assert_eq!(
            choose(&probe(0.75, 0.3, 0.2, 3.0, false)),
            Ok(TraverseAction::Hurdle)
        );
        assert_eq!(
            choose(&probe(1.25, 0.6, 0.2, 2.0, false)),
            Ok(TraverseAction::Vault)
        );
        assert_eq!(
            choose(&probe(1.7, 0.6, 0.2, 2.0, false)),
            Ok(TraverseAction::Mantle)
        );
        assert_eq!(
            choose(&probe(2.6, 0.4, 0.2, 1.0, true)),
            Ok(TraverseAction::Climb)
        );
        assert_eq!(
            choose(&probe(-0.3, 0.4, 0.2, 1.0, false)),
            Ok(TraverseAction::Drop)
        );
    }

    #[test]
    fn slow_or_wide_low_blocks_become_mantles() {
        assert_eq!(
            choose(&probe(0.5, 0.3, 0.2, 0.5, false)),
            Ok(TraverseAction::Mantle)
        );
        assert_eq!(
            choose(&probe(0.5, 2.0, 0.2, 3.0, false)),
            Ok(TraverseAction::Mantle)
        );
        assert_eq!(
            choose(&probe(1.0, 2.5, 0.2, 2.0, false)),
            Ok(TraverseAction::Mantle)
        );
    }

    #[test]
    fn gap_too_wide_refuses() {
        assert_eq!(
            choose(&probe(1.0, 0.6, 2.6, 2.0, false)),
            Err(TraverseError::GapTooWide {
                gap_mm: 2600,
                max_mm: MAX_GAP_MM
            })
        );
        assert!(choose(&probe(1.0, 0.6, 2.5, 2.0, false)).is_ok());
    }

    #[test]
    fn climb_without_hold_refuses() {
        assert_eq!(
            choose(&probe(2.0, 0.4, 0.3, 1.0, false)),
            Err(TraverseError::NoHold)
        );
        assert_eq!(
            choose(&probe(2.0, 0.4, 0.3, 1.0, true)),
            Ok(TraverseAction::Climb)
        );
    }

    #[test]
    fn bad_probes_name_the_lane() {
        assert_eq!(
            choose(&probe(f32::NAN, 0.5, 0.2, 1.0, false)),
            Err(TraverseError::BadProbe("height"))
        );
        assert_eq!(
            choose(&probe(1.0, -0.5, 0.2, 1.0, false)),
            Err(TraverseError::BadProbe("depth"))
        );
        assert_eq!(
            choose(&probe(1.0, 0.5, f32::INFINITY, 1.0, false)),
            Err(TraverseError::BadProbe("gap"))
        );
        assert_eq!(
            choose(&probe(1.0, 0.5, 0.2, 99.0, false)),
            Err(TraverseError::BadProbe("speed"))
        );
        assert_eq!(
            choose(&probe(3.5, 0.5, 0.2, 1.0, true)),
            Err(TraverseError::BadProbe("height"))
        );
        assert_eq!(
            choose(&probe(-3.0, 0.5, 0.2, 1.0, false)),
            Err(TraverseError::BadProbe("drop"))
        );
    }

    #[test]
    fn duration_table_is_golden() {
        assert_eq!(
            [
                duration_ticks(TraverseAction::None),
                duration_ticks(TraverseAction::Hurdle),
                duration_ticks(TraverseAction::Vault),
                duration_ticks(TraverseAction::Mantle),
                duration_ticks(TraverseAction::Climb),
                duration_ticks(TraverseAction::Drop),
            ],
            [0, 20, 30, 40, 60, 25]
        );
    }

    #[test]
    fn warp_lands_on_edge_within_1cm() {
        let cases = [
            probe(0.5, 0.3, 0.4, 3.0, false),
            probe(1.0, 0.6, 0.5, 2.0, false),
            probe(1.5, 2.0, 0.2, 1.0, false),
            probe(2.0, 0.4, 0.3, 1.0, true),
            probe(-1.0, 0.5, 0.4, 1.5, false),
        ];
        let edge = [4.0, 1.0];
        let mut worst = 0.0f32;
        for probe in cases {
            let plan = plan(&probe, edge).expect("fixture plans");
            let raw = [[0.0, 0.0], [1.0, 0.25], [2.0, 0.5]];
            let warped = apply_warp(&plan, &raw).expect("fixture warps");
            let end = *warped.last().expect("warped nonempty");
            let err = warp_error_mm(end, edge);
            worst = if err > worst { err } else { worst };
            assert!(err <= WARP_TOL_MM, "miss {err} mm past 1 cm");
            assert_eq!(err, 0.0, "integer warp must land exactly");
        }
        assert_eq!(worst, 0.0);
    }

    #[test]
    fn warp_preserves_shape() {
        let plan = plan(&probe(1.0, 0.6, 0.5, 2.0, false), [4.0, 1.0]).expect("fixture plans");
        let raw = [[0.0, 0.0], [1.0, 0.5], [2.5, -0.5]];
        let warped = apply_warp(&plan, &raw).expect("fixture warps");
        for window in 0..raw.len() - 1 {
            for lane in 0..2 {
                let raw_step = raw[window + 1][lane] - raw[window][lane];
                let warped_step = warped[window + 1][lane] - warped[window][lane];
                assert!(
                    (warped_step - raw_step).abs() < 1e-6,
                    "warp must translate only"
                );
            }
        }
    }

    #[test]
    fn warp_rejects_bad_paths() {
        let committed = plan(&probe(1.0, 0.6, 0.5, 2.0, false), [4.0, 1.0]).expect("fixture plans");
        assert_eq!(
            apply_warp(&committed, &[]),
            Err(TraverseError::BadProbe("path empty"))
        );
        assert_eq!(
            apply_warp(&committed, &[[f32::NAN, 0.0]]),
            Err(TraverseError::BadProbe("path"))
        );
        let long = vec![[0.0, 0.0]; MAX_PATH + 1];
        assert_eq!(
            apply_warp(&committed, &long),
            Err(TraverseError::BadProbe("path too long"))
        );
        assert_eq!(
            plan(&probe(1.0, 0.6, 0.5, 2.0, false), [f32::INFINITY, 0.0]),
            Err(TraverseError::BadProbe("edge"))
        );
    }

    #[test]
    fn replay_is_exact() {
        let fixture = probe(1.0, 0.6, 0.5, 2.0, false);
        let edge = [4.0, 1.0];
        let first = plan(&fixture, edge).expect("fixture plans");
        let second = plan(&fixture, edge).expect("fixture plans");
        assert_eq!(first, second);
        assert_eq!(plan_hash(&first), plan_hash(&second));
        let raw = [[0.0, 0.0], [1.0, 0.25], [2.0, 0.5]];
        let warped_first = apply_warp(&first, &raw).expect("fixture warps");
        let warped_second = apply_warp(&second, &raw).expect("fixture warps");
        assert_eq!(warped_first, warped_second);
        let mut moved = fixture;
        moved.speed_m_s = 5.0;
        assert_ne!(
            plan_hash(&plan(&moved, edge).expect("plans")),
            plan_hash(&first)
        );
    }

    #[test]
    fn vault_mantle_climb_params_are_golden() {
        assert_eq!(
            action_params(TraverseAction::Vault),
            MoveParams {
                duration_ticks: 30,
                min_approach_mm_s: HURDLE_MIN_SPEED_MM_S,
                max_depth_mm: VAULT_DEPTH_MM,
                needs_hold: false,
            }
        );
        assert_eq!(
            action_params(TraverseAction::Mantle),
            MoveParams {
                duration_ticks: 40,
                min_approach_mm_s: 0,
                max_depth_mm: i32::MAX,
                needs_hold: false,
            }
        );
        assert_eq!(
            action_params(TraverseAction::Climb),
            MoveParams {
                duration_ticks: 60,
                min_approach_mm_s: 0,
                max_depth_mm: i32::MAX,
                needs_hold: true,
            }
        );
        assert_eq!(
            action_params(TraverseAction::Hurdle),
            MoveParams {
                duration_ticks: 20,
                min_approach_mm_s: HURDLE_MIN_SPEED_MM_S,
                max_depth_mm: HURDLE_DEPTH_MM,
                needs_hold: false,
            }
        );
        assert_eq!(
            action_params(TraverseAction::Drop),
            MoveParams {
                duration_ticks: 25,
                min_approach_mm_s: 0,
                max_depth_mm: 0,
                needs_hold: false,
            }
        );
        assert_eq!(
            action_params(TraverseAction::None),
            MoveParams {
                duration_ticks: 0,
                min_approach_mm_s: 0,
                max_depth_mm: 0,
                needs_hold: false,
            }
        );
    }

    #[test]
    fn committed_durations_ride_the_params() {
        let cases = [
            (probe(0.0, 0.0, 0.0, 1.5, false), TraverseAction::None),
            (probe(0.5, 0.3, 0.4, 3.0, false), TraverseAction::Hurdle),
            (probe(1.0, 0.6, 0.5, 2.0, false), TraverseAction::Vault),
            (probe(1.5, 2.0, 0.2, 1.0, false), TraverseAction::Mantle),
            (probe(2.0, 0.4, 0.3, 1.0, true), TraverseAction::Climb),
            (probe(-1.0, 0.5, 0.4, 1.5, false), TraverseAction::Drop),
        ];
        let edge = [4.0, 1.0];
        for (fixture, want) in cases {
            let committed = plan(&fixture, edge).expect("fixture plans");
            assert_eq!(committed.action, want);
            assert_eq!(committed.duration_ticks, action_params(want).duration_ticks);
        }
    }

    #[test]
    fn loco_gate_grounds_air_moves() {
        for action in [
            TraverseAction::None,
            TraverseAction::Hurdle,
            TraverseAction::Vault,
            TraverseAction::Mantle,
            TraverseAction::Climb,
            TraverseAction::Drop,
        ] {
            assert_eq!(loco_gate(action, 1.5, false), Ok(()));
        }
        for action in [
            TraverseAction::None,
            TraverseAction::Hurdle,
            TraverseAction::Vault,
            TraverseAction::Mantle,
            TraverseAction::Climb,
        ] {
            assert_eq!(
                loco_gate(action, 1.5, true),
                Err(TraverseError::BadProbe("airborne"))
            );
        }
        assert_eq!(loco_gate(TraverseAction::Drop, 1.5, true), Ok(()));
        assert_eq!(
            loco_gate(TraverseAction::Vault, f32::NAN, false),
            Err(TraverseError::BadProbe("speed"))
        );
        assert_eq!(
            loco_gate(TraverseAction::Vault, -1.0, false),
            Err(TraverseError::BadProbe("speed"))
        );
        assert_eq!(
            loco_gate(TraverseAction::Vault, PROBE_SPEED_RANGE_M_S + 1.0, false),
            Err(TraverseError::BadProbe("speed"))
        );
    }

    #[test]
    fn approach_query_shapes_contact_from_action() {
        use crate::motion_match::NO_HIST;
        assert_eq!(
            approach_query(
                &probe(f32::NAN, 0.5, 0.2, 1.0, false),
                TraverseAction::Vault,
                NO_HIST
            ),
            Err(TraverseError::BadProbe("height"))
        );
        let vault = approach_query(
            &probe(1.0, 0.6, 0.5, 2.0, false),
            TraverseAction::Vault,
            NO_HIST,
        )
        .expect("fixture query valid");
        assert_eq!(vault.contact, 3);
        let drop = approach_query(
            &probe(-1.0, 0.5, 0.4, 1.5, false),
            TraverseAction::Drop,
            NO_HIST,
        )
        .expect("drop query valid");
        assert_eq!(drop.contact, 0);
    }

    /// Test-only brute force over a slice with the built query's own lanes:
    /// the ranked pick must equal it.
    fn brute_slice(
        db: &crate::motion_match::MotionDb,
        query: &crate::motion_match::Query,
        start: usize,
        end: usize,
    ) -> (usize, i64) {
        let mut best = i64::MAX;
        let mut at = start;
        for to in start..end {
            let entry = &db.entries[to];
            let mut sum = query
                .contact_cost
                .saturating_mul(i64::from(entry.contact != query.contact));
            for lane in 0..crate::motion_match::FEAT_N {
                let diff = i64::from(entry.feat[lane]) - i64::from(query.feat[lane]);
                sum = sum.saturating_add(
                    diff.saturating_mul(diff)
                        .saturating_mul(i64::from(query.weights[lane])),
                );
            }
            if sum < best {
                best = sum;
                at = to;
            }
        }
        (at, best)
    }

    #[test]
    fn rank_approach_matches_brute_force_in_slice() {
        use super::motiondb::DbSlice;
        use crate::locomotion::Locomotion;
        use crate::motion_match::{bake, make_feature, predict_linear, Entry, PoseSample, NO_HIST};
        fn db_entry(clip: u16, tick: u16, hip: f32, contact: u8) -> Entry {
            let sample = PoseSample {
                hip_vel: [0.0, hip],
                foot_l: [0.11, 0.05, 0.0],
                foot_r: [-0.11, 0.05, 0.0],
                contact,
            };
            let traj = predict_linear([0.0, 0.0], sample.hip_vel).expect("finite");
            Entry {
                feat: make_feature(&sample, &traj).expect("valid"),
                clip,
                tick,
                contact,
            }
        }
        let db = bake(vec![
            db_entry(0, 0, 0.0, 3),
            db_entry(0, 1, 1.0, 3),
            db_entry(0, 2, 2.0, 3),
            db_entry(1, 0, 3.0, 1),
            db_entry(1, 1, 4.0, 0),
            db_entry(1, 2, 2.0, 0),
        ])
        .expect("fixture bakes");
        let vault_probe = probe(1.0, 0.6, 0.5, 2.0, false);
        let loco = Locomotion::new();
        let slice = DbSlice::new(0, 6).expect("whole window");
        let query = approach_query(&vault_probe, TraverseAction::Vault, NO_HIST)
            .expect("fixture query valid");
        let (got, receipt) = rank_approach(
            &db,
            &vault_probe,
            TraverseAction::Vault,
            &loco,
            slice,
            NO_HIST,
        )
        .expect("ranks");
        assert_eq!((got.index, got.cost), brute_slice(&db, &query, 0, 6));
        assert_eq!(receipt.total, 6);
        assert_eq!(receipt.full_evals + receipt.early_cuts, receipt.total);
        // Identical ranks twice on the same inputs.
        assert_eq!(
            rank_approach(
                &db,
                &vault_probe,
                TraverseAction::Vault,
                &loco,
                slice,
                NO_HIST
            )
            .expect("repeat"),
            (got, receipt)
        );
        // A head window confines the pick even when the global best sits past it.
        let head = DbSlice::new(0, 2).expect("head window");
        let (held, _) = rank_approach(
            &db,
            &vault_probe,
            TraverseAction::Vault,
            &loco,
            head,
            NO_HIST,
        )
        .expect("ranks in window");
        assert!(held.index < 2, "pick escapes the window: {held:?}");
        assert_eq!((held.index, held.cost), brute_slice(&db, &query, 0, 2));
    }

    #[test]
    fn rank_approach_refuses_bad_links() {
        use super::motiondb::DbSlice;
        use crate::locomotion::Locomotion;
        use crate::motion_match::{bake, NO_HIST};
        let db = bake(vec![crate::motion_match::Entry {
            feat: [0; crate::motion_match::FEAT_N],
            clip: 0,
            tick: 0,
            contact: 3,
        }])
        .expect("fixture bakes");
        let vault_probe = probe(1.0, 0.6, 0.5, 2.0, false);
        let mut air = Locomotion::new();
        air.set_airborne(true);
        assert_eq!(
            rank_approach(
                &db,
                &vault_probe,
                TraverseAction::Vault,
                &air,
                DbSlice::new(0, 1).expect("window"),
                NO_HIST
            ),
            Err(TraverseError::BadProbe("airborne"))
        );
        let loco = Locomotion::new();
        assert_eq!(
            rank_approach(
                &db,
                &vault_probe,
                TraverseAction::Vault,
                &loco,
                DbSlice::new(0, 2).expect("window"),
                NO_HIST
            ),
            Err(TraverseError::BadProbe("past end"))
        );
        assert_eq!(
            rank_approach(
                &crate::motion_match::MotionDb::default(),
                &vault_probe,
                TraverseAction::Vault,
                &loco,
                DbSlice::new(0, 1).expect("window"),
                NO_HIST
            ),
            Err(TraverseError::BadProbe("empty database"))
        );
        assert_eq!(
            rank_approach(
                &db,
                &probe(f32::NAN, 0.5, 0.2, 1.0, false),
                TraverseAction::Vault,
                &loco,
                DbSlice::new(0, 1).expect("window"),
                NO_HIST
            ),
            Err(TraverseError::BadProbe("height"))
        );
    }
}
