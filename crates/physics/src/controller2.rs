//! controller2: character controller v2 — steps, slopes, moving platforms,
//! ledge sense feeding traversal, fighter push and crowd shove.
//!
//! Consumer: arena locomotion walks these walkers; the ledge probes mirror
//! the traversal thresholds by value (verified 2026-10-16: no `engine-anim`
//! consumer, `crates/anim` holds zero `engine-physics` refs). Data owner: `engine-physics`; this module never imports assets,
//! arena or the render path. Units: integer millimeters, ticks at 60 Hz.
//!
//! Determinism scope: the authority path uses only `+ - * /` on `i32`/`i64`
//! with truncation toward zero and a fixed iteration order over caller
//! slices, so the same world plus the same inputs replay byte-identically on
//! every PC. No floats, no `sin`/`cos`/`sqrt`, no wall clock, no hash-map
//! order. Pass world slices in a stable (for example id-sorted) order.
//! Errors are [`ProbeError`]; per-tick input is clamped, never panics.
//!
//! Donor shapes (reimplemented, never vendored): `bevy-tnua` step / slope /
//! coyote-buffer timing plus moving-platform ride and ledge detect; the
//! ledge table mirrors the traversal `choose` thresholds without depending on
//! `engine-anim`, so the default headless build stays dependency-free.

#![forbid(unsafe_code)]

use crate::material::{Material, GROUND};
use engine_core::Fnv64;

/// Ticks per second; every rate below divides by this with truncation.
pub const TICKS_PER_S: i32 = 60;
/// Tallest rise the walker steps onto without a traversal move, millimeters.
/// Anything at or below the traversal step-through bound needs no ledge action.
pub const STEP_HEIGHT_MM: i32 = 500;
/// Farthest the walker sticks to falling ground while still grounded.
pub const SNAP_MM: i32 = 400;
/// Walkable slope cap, rise per mille of run; 1000 is 45 degrees.
pub const MAX_SLOPE_GRADE_Q: i32 = 1000;
/// Gravity pull, millimeters per second squared.
pub const GRAVITY_MM_S2: i32 = 22_000;
/// Fastest allowed fall speed, millimeters per second.
pub const MAX_FALL_MM_S: i32 = -20_000;
/// Takeoff speed of a buffered jump, millimeters per second.
pub const JUMP_V_MM_S: i32 = 8_000;
/// Horizontal acceleration while grounded, millimeters per second squared.
pub const ACCEL_GROUND_MM_S2: i32 = 60_000;
/// Horizontal acceleration while airborne, millimeters per second squared.
pub const ACCEL_AIR_MM_S2: i32 = 20_000;
/// Largest accepted wish speed per lane, millimeters per second.
pub const MAX_WISH_MM_S: i32 = 8_000;
/// Downhill slide speed on over-steep ground, millimeters per second.
pub const SLIP_MM_S: i32 = 4_000;
/// Grace ticks after leaving walkable ground that still allow a jump.
pub const COYOTE_TICKS: u32 = 9;
/// Ticks a jump press waits for landing or coyote ground.
pub const BUFFER_TICKS: u32 = 9;
/// Walker collision radius, millimeters.
pub const RADIUS_MM: i32 = 350;
/// Walker collision height, millimeters.
pub const HEIGHT_MM: i32 = 1_700;
/// Strongest crowd shove impulse, millimeters per second.
pub const MAX_SHOVE_MM_S: i32 = 8_000;
/// Ledge table, mirrored from the traversal chooser so probes agree by value.
/// Below this rise the actor steps through with no traversal move.
pub const TV1_STEP_MM: i32 = 200;
/// Tallest top cleared as a hurdle.
pub const TV1_HURDLE_MAX_MM: i32 = 750;
/// Tallest top cleared as a vault.
pub const TV1_VAULT_MAX_MM: i32 = 1_250;
/// Tallest top cleared as a mantle.
pub const TV1_MANTLE_MAX_MM: i32 = 1_700;
/// Tallest top cleared as a climb (needs a hold).
pub const TV1_CLIMB_MAX_MM: i32 = 2_600;
/// Widest obstacle hurdled; wider low blocks are mantled onto.
pub const TV1_HURDLE_DEPTH_MM: i32 = 800;
/// Widest obstacle vaulted; wider chest blocks are mantled onto.
pub const TV1_VAULT_DEPTH_MM: i32 = 1_200;
/// Slow approaches over hurdle-high rails are mantled, not hurdled.
pub const TV1_HURDLE_MIN_SPEED_MM_S: i32 = 1_000;
/// Drops start past this depth below the feet.
pub const TV1_DROP_EDGE_MM: i32 = -300;
/// Deeper drops than this are refused, never silently clamped.
pub const TV1_DROP_MAX_MM: i32 = -2_500;
/// Widest clearable gap; wider probes refuse.
pub const TV1_MAX_GAP_MM: i32 = 2_500;
/// Probe height lane, millimeters above or below the feet.
pub const PROBE_HEIGHT_RANGE_MM: i32 = 5_000;
/// Probe depth and gap lane cap, millimeters.
pub const PROBE_SPAN_RANGE_MM: i32 = 5_000;
/// Fastest accepted approach speed, millimeters per second.
pub const PROBE_SPEED_RANGE_MM_S: i32 = 12_000;
/// Farthest gap march when sensing open ground ahead, millimeters.
pub const SENSE_RANGE_MM: i32 = 5_000;
/// Gap march stride, millimeters.
pub const SENSE_STRIDE_MM: i32 = 100;

/// Per-tick tuning; every lane is integer millimeters or ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Tallest auto-step rise in millimeters.
    pub step_height_mm: i32,
    /// Ground snap distance in millimeters.
    pub snap_mm: i32,
    /// Walkable slope cap, rise per mille of run.
    pub max_slope_grade_q: i32,
    /// Horizontal ground acceleration, millimeters per second squared.
    pub accel_ground_mm_s2: i32,
    /// Horizontal air acceleration, millimeters per second squared.
    pub accel_air_mm_s2: i32,
    /// Wish clamp per lane, millimeters per second.
    pub max_wish_mm_s: i32,
    /// Jump takeoff speed, millimeters per second.
    pub jump_v_mm_s: i32,
    /// Grip of the ground underfoot, thousandths: ground acceleration scales with it against
    /// the [`GROUND`] row, so the default ground changes nothing.
    pub friction_pm: i32,
}

impl Config {
    /// The tune for a walker standing on `ground`: its step height and its grip come straight
    /// from the material table, the one source `engine-physfx` reads its contact friction from.
    pub fn on(ground: Material) -> Self {
        let row = ground.props();
        Self {
            step_height_mm: i32::from(row.step_mm),
            snap_mm: SNAP_MM,
            max_slope_grade_q: MAX_SLOPE_GRADE_Q,
            accel_ground_mm_s2: ACCEL_GROUND_MM_S2,
            accel_air_mm_s2: ACCEL_AIR_MM_S2,
            max_wish_mm_s: MAX_WISH_MM_S,
            jump_v_mm_s: JUMP_V_MM_S,
            friction_pm: i32::from(row.friction_pm),
        }
    }

    /// Horizontal acceleration while grounded, millimeters per second squared: the base lane
    /// scaled by this ground's grip over the [`GROUND`] row's (integers, truncating).
    pub fn grip_accel_mm_s2(&self) -> i32 {
        let reference = i64::from(GROUND.props().friction_pm);
        let scaled = i64::from(self.accel_ground_mm_s2.max(0)) * i64::from(self.friction_pm.max(0))
            / reference;
        i32::try_from(scaled).unwrap_or(i32::MAX)
    }
}

impl Default for Config {
    /// Default tune: the [`GROUND`] row (0.5 m steps, the base grip), 45-degree slopes,
    /// Tnua-style timers.
    fn default() -> Self {
        Self::on(GROUND)
    }
}

/// Static solid box, integer millimeters; the constructor orders each lane
/// so callers never hand the stepper an inverted box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    /// Minimum corner, `[x, y, z]` millimeters.
    pub min_mm: [i32; 3],
    /// Maximum corner, `[x, y, z]` millimeters.
    pub max_mm: [i32; 3],
}

impl Block {
    /// New box with each lane ordered min-to-max.
    pub fn new(min_mm: [i32; 3], max_mm: [i32; 3]) -> Self {
        Self {
            min_mm: [
                min_mm[0].min(max_mm[0]),
                min_mm[1].min(max_mm[1]),
                min_mm[2].min(max_mm[2]),
            ],
            max_mm: [
                min_mm[0].max(max_mm[0]),
                min_mm[1].max(max_mm[1]),
                min_mm[2].max(max_mm[2]),
            ],
        }
    }
}

/// Walkable or over-steep ramp rising along +x from `x0_mm` to `x1_mm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ramp {
    /// Low-x edge of the ramp, millimeters.
    pub x0_mm: i32,
    /// High-x edge of the ramp, millimeters.
    pub x1_mm: i32,
    /// Z span low edge, millimeters.
    pub z0_mm: i32,
    /// Z span high edge, millimeters.
    pub z1_mm: i32,
    /// Surface height at `x0_mm`, millimeters.
    pub y_base_mm: i32,
    /// Rise added at `x1_mm`, millimeters (may be negative downhill).
    pub rise_mm: i32,
}

impl Ramp {
    /// Signed run from `x0_mm` to `x1_mm`; a ramp authored high-x first keeps its slope sign
    /// (a `max(1)` clamp turned it into a one-millimetre cliff). Only a zero run is degenerate.
    fn run(&self) -> i64 {
        let run = i64::from(self.x1_mm) - i64::from(self.x0_mm);
        if run == 0 {
            1
        } else {
            run
        }
    }

    /// Surface height under `(x_mm, _)`; callers check the XZ rect first.
    fn surface_at(&self, x_mm: i32) -> i32 {
        let num = i64::from(self.rise_mm) * (i64::from(x_mm) - i64::from(self.x0_mm));
        narrow(self.y_base_mm, num / self.run())
    }

    /// Grade in rise per mille of run along +x (negative descends toward +x).
    fn grade_q(&self) -> i32 {
        let num = i64::from(self.rise_mm) * 1_000;
        narrow(0, num / self.run())
    }

    /// True when `(x_mm, z_mm)` sits on the ramp rect.
    fn contains(&self, x_mm: i32, z_mm: i32) -> bool {
        x_mm >= self.x0_mm.min(self.x1_mm)
            && x_mm <= self.x0_mm.max(self.x1_mm)
            && z_mm >= self.z0_mm.min(self.z1_mm)
            && z_mm <= self.z0_mm.max(self.z1_mm)
    }
}

/// Moving platform slab; `delta_mm` is this tick's travel and the caller
/// advances `min_mm`/`max_mm` by it after the step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Platform {
    /// Stable platform id; riders name it in [`Walker::ground_platform`].
    pub id: u32,
    /// Minimum corner, `[x, y, z]` millimeters.
    pub min_mm: [i32; 3],
    /// Maximum corner, `[x, y, z]` millimeters.
    pub max_mm: [i32; 3],
    /// Travel applied to riders this tick, `[x, y, z]` millimeters.
    pub delta_mm: [i32; 3],
}

/// Borrowed step world; slices stay caller-owned so the tick allocates.
#[derive(Clone, Copy, Debug)]
pub struct StepWorld<'a> {
    /// Static solids in stable order.
    pub blocks: &'a [Block],
    /// Ramps in stable order.
    pub ramps: &'a [Ramp],
    /// Platforms in stable order.
    pub platforms: &'a [Platform],
}

/// Walking fighter: feet position, velocity, and jump timers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Walker {
    /// Canonical fighter id; breaks push ties deterministically.
    pub id: u32,
    /// Feet position, `[x, y, z]` millimeters.
    pub pos_mm: [i32; 3],
    /// Velocity, `[x, y, z]` millimeters per second.
    pub vel_mm_s: [i32; 3],
    /// True while resting on walkable support.
    pub grounded: bool,
    /// Platform id being ridden, if any.
    pub ground_platform: Option<u32>,
    /// Remaining coyote ticks.
    pub coyote_ticks: u32,
    /// Remaining jump-buffer ticks.
    pub buffer_ticks: u32,
    /// Travel under one millimetre carried to the next tick, `[x, y, z]` in mm/s units
    /// (`|lane| < TICKS_PER_S`); see [`travel`].
    pub carry: [i32; 3],
}

impl Walker {
    /// New standing walker with zero velocity and full timers unset.
    pub fn spawn(id: u32, pos_mm: [i32; 3]) -> Self {
        Self {
            id,
            pos_mm,
            vel_mm_s: [0, 0, 0],
            grounded: false,
            ground_platform: None,
            coyote_ticks: 0,
            buffer_ticks: 0,
            carry: [0, 0, 0],
        }
    }
}

/// One tick of travel at `vel_mm_s`, in millimetres, with the remainder kept in `carry`.
///
/// Plain `vel / TICKS_PER_S` stalls every speed under 60 mm/s and shaves up to 59 mm/s off the
/// rest (100 mm/s walked at 60). Carrying the remainder moves exactly `vel` millimetres per
/// second on average at any speed. Truncation toward zero keeps it mirror-symmetric, and a
/// walker at rest never moves: `|carry| < TICKS_PER_S` alone rounds to zero.
fn travel(vel_mm_s: i32, carry: &mut i32) -> i64 {
    let total = i64::from(vel_mm_s) + i64::from(*carry);
    let ticks = i64::from(TICKS_PER_S);
    let step = total / ticks;
    *carry = i32::try_from(total - step * ticks).unwrap_or(0);
    step
}

/// Per-tick driver input; lanes clamp to [`MAX_WISH_MM_S`] by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickInput {
    /// Wish horizontal velocity, `[x, z]` millimeters per second.
    pub wish_mm_s: [i32; 2],
    /// True on the tick the jump button goes down.
    pub jump_pressed: bool,
}

impl TickInput {
    /// Idle input: no wish, no jump.
    pub fn idle() -> Self {
        Self {
            wish_mm_s: [0, 0],
            jump_pressed: false,
        }
    }
}

/// Per-tick resolution report: what the walker asked against what moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// Horizontal displacement actually applied, `[x, z]` millimeters.
    pub displacement_mm: [i32; 2],
    /// Vertical displacement actually applied, millimeters.
    pub vertical_mm: i32,
    /// True while resting on walkable support after this tick.
    pub grounded: bool,
    /// Outward wall normal `[x, z]` when a horizontal axis blocked.
    pub wall_normal: Option<[i32; 2]>,
    /// True when the walker stepped up a stair or ledge this tick.
    pub stepped: bool,
    /// True when over-steep ground pushed the walker downhill this tick.
    pub slipped: bool,
    /// Platform id ridden into this tick, if any.
    pub rode_platform: Option<u32>,
}

/// Traversal move the ledge table commits to; values mirror the traversal chooser.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LedgeAction {
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

/// Ledge probe failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProbeError {
    /// A probe lane is out of bounds; the lane names it.
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

impl core::fmt::Display for ProbeError {
    /// Human-readable probe failure.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ProbeError::BadProbe(lane) => write!(f, "controller2: bad probe ({lane})"),
            ProbeError::NoHold => write!(f, "controller2: climb needs a hold"),
            ProbeError::GapTooWide { gap_mm, max_mm } => {
                write!(f, "controller2: gap {gap_mm} mm past {max_mm} mm")
            }
        }
    }
}

impl std::error::Error for ProbeError {}

/// Ledge sense ahead of the walker; `classify` turns it into a move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AheadSense {
    /// Support height at the probed point, if any lies within step reach.
    pub support_mm: Option<i32>,
    /// Lowest solid top above step reach at the probed point, if any.
    pub wall_mm: Option<i32>,
    /// Distance marched to the next support, capped at [`SENSE_RANGE_MM`].
    pub gap_mm: i32,
}

impl AheadSense {
    /// Drop below the feet at the probed point; `None` means solid or void
    /// without a measured shelf, never a guessed zero.
    pub fn drop_mm(&self, feet_y_mm: i32) -> Option<i32> {
        self.support_mm.map(|s| s - feet_y_mm)
    }
}

/// Fixed ticks per move at 60 Hz; matches the traversal move durations.
pub fn duration_ticks(action: LedgeAction) -> u32 {
    match action {
        LedgeAction::None => 0,
        LedgeAction::Hurdle => 20,
        LedgeAction::Vault => 30,
        LedgeAction::Mantle => 40,
        LedgeAction::Climb => 60,
        LedgeAction::Drop => 25,
    }
}

/// Choose the traversal move for integer-millimeter probe lanes. Gap refusal
/// comes first so no move is ever picked past what any of them can clear.
/// Thresholds match the traversal chooser lane for lane so probes handed to
/// `engine-anim` pick the same move there.
pub fn classify(
    height_mm: i32,
    depth_mm: i32,
    gap_mm: i32,
    speed_mm_s: i32,
    has_hold: bool,
) -> Result<LedgeAction, ProbeError> {
    if !(-PROBE_HEIGHT_RANGE_MM..=PROBE_HEIGHT_RANGE_MM).contains(&height_mm) {
        return Err(ProbeError::BadProbe("height"));
    }
    if !(0..=PROBE_SPAN_RANGE_MM).contains(&depth_mm) {
        return Err(ProbeError::BadProbe("depth"));
    }
    if !(0..=PROBE_SPAN_RANGE_MM).contains(&gap_mm) {
        return Err(ProbeError::BadProbe("gap"));
    }
    if !(0..=PROBE_SPEED_RANGE_MM_S).contains(&speed_mm_s) {
        return Err(ProbeError::BadProbe("speed"));
    }
    if gap_mm > TV1_MAX_GAP_MM {
        return Err(ProbeError::GapTooWide {
            gap_mm,
            max_mm: TV1_MAX_GAP_MM,
        });
    }
    if height_mm <= TV1_DROP_EDGE_MM {
        if height_mm < TV1_DROP_MAX_MM {
            return Err(ProbeError::BadProbe("drop"));
        }
        return Ok(LedgeAction::Drop);
    }
    if height_mm < TV1_STEP_MM {
        return Ok(LedgeAction::None);
    }
    if height_mm <= TV1_HURDLE_MAX_MM {
        if depth_mm <= TV1_HURDLE_DEPTH_MM && speed_mm_s >= TV1_HURDLE_MIN_SPEED_MM_S {
            return Ok(LedgeAction::Hurdle);
        }
        return Ok(LedgeAction::Mantle);
    }
    if height_mm <= TV1_VAULT_MAX_MM {
        if depth_mm <= TV1_VAULT_DEPTH_MM {
            return Ok(LedgeAction::Vault);
        }
        return Ok(LedgeAction::Mantle);
    }
    if height_mm <= TV1_MANTLE_MAX_MM {
        return Ok(LedgeAction::Mantle);
    }
    if height_mm <= TV1_CLIMB_MAX_MM {
        if has_hold {
            return Ok(LedgeAction::Climb);
        }
        return Err(ProbeError::NoHold);
    }
    Err(ProbeError::BadProbe("height"))
}

/// Saturating narrow: `base + delta` clamped into `i32` without float casts.
fn narrow(base: i32, delta: i64) -> i32 {
    let sum = i64::from(base) + delta;
    i32::try_from(sum).unwrap_or(if sum < 0 { i32::MIN } else { i32::MAX })
}

/// True when the walker column at `(x_mm, z_mm, feet_y_mm)` overlaps `box_`.
fn column_hits_box(x_mm: i32, z_mm: i32, feet_y_mm: i32, box_: &Block) -> bool {
    x_mm >= box_.min_mm[0] - RADIUS_MM
        && x_mm <= box_.max_mm[0] + RADIUS_MM
        && z_mm >= box_.min_mm[2] - RADIUS_MM
        && z_mm <= box_.max_mm[2] + RADIUS_MM
        && feet_y_mm < box_.max_mm[1]
        && feet_y_mm + HEIGHT_MM > box_.min_mm[1]
}

/// True when the body column above `base_y_mm` is free of static boxes.
fn column_free(x_mm: i32, z_mm: i32, base_y_mm: i32, world: &StepWorld<'_>) -> bool {
    for box_ in world.blocks {
        if x_mm >= box_.min_mm[0] - RADIUS_MM
            && x_mm <= box_.max_mm[0] + RADIUS_MM
            && z_mm >= box_.min_mm[2] - RADIUS_MM
            && z_mm <= box_.max_mm[2] + RADIUS_MM
            && base_y_mm < box_.max_mm[1]
            && base_y_mm + HEIGHT_MM > box_.min_mm[1]
        {
            return false;
        }
    }
    true
}

/// Highest walkable support under `(x_mm, z_mm)` within step reach above
/// `feet_y_mm`; the max over candidates is slice-order independent.
fn support_at(
    x_mm: i32,
    z_mm: i32,
    feet_y_mm: i32,
    world: &StepWorld<'_>,
    cfg: &Config,
) -> (Option<i32>, Option<u32>) {
    let cap = feet_y_mm + cfg.step_height_mm;
    let mut best: Option<i32> = None;
    let mut ride: Option<u32> = None;
    for box_ in world.blocks {
        if x_mm >= box_.min_mm[0] - RADIUS_MM
            && x_mm <= box_.max_mm[0] + RADIUS_MM
            && z_mm >= box_.min_mm[2] - RADIUS_MM
            && z_mm <= box_.max_mm[2] + RADIUS_MM
            && box_.max_mm[1] <= cap
            && best.is_none_or(|b| box_.max_mm[1] > b)
        {
            best = Some(box_.max_mm[1]);
            ride = None;
        }
    }
    for ramp in world.ramps {
        if !ramp.contains(x_mm, z_mm) {
            continue;
        }
        let grade = ramp.grade_q();
        if grade < -cfg.max_slope_grade_q || grade > cfg.max_slope_grade_q {
            continue;
        }
        let surface = ramp.surface_at(x_mm);
        if surface <= cap && best.is_none_or(|b| surface > b) {
            best = Some(surface);
            ride = None;
        }
    }
    for platform in world.platforms {
        if x_mm >= platform.min_mm[0] - RADIUS_MM
            && x_mm <= platform.max_mm[0] + RADIUS_MM
            && z_mm >= platform.min_mm[2] - RADIUS_MM
            && z_mm <= platform.max_mm[2] + RADIUS_MM
            && platform.max_mm[1] <= cap
            && best.is_none_or(|b| platform.max_mm[1] >= b)
        {
            // Ties prefer platforms so a rider standing exactly on a deck
            // attaches to the platform id instead of the slab beneath it.
            best = Some(platform.max_mm[1]);
            ride = Some(platform.id);
        }
    }
    (best, ride)
}

/// Over-steep ramp surface underfoot, if the feet rest on one, with the downhill x sign.
///
/// Both slope signs count: a steep ramp that descends toward +x was walkable by nobody and
/// solid to nobody (`support_at` skips it as too steep), so walkers fell through it.
fn steep_surface_at(
    x_mm: i32,
    z_mm: i32,
    world: &StepWorld<'_>,
    cfg: &Config,
) -> Option<(i32, i32)> {
    for ramp in world.ramps {
        if !ramp.contains(x_mm, z_mm) {
            continue;
        }
        let grade = ramp.grade_q();
        if grade > cfg.max_slope_grade_q || grade < -cfg.max_slope_grade_q {
            return Some((ramp.surface_at(x_mm), -grade.signum()));
        }
    }
    None
}

/// Sense the ground ahead: support, wall and gap at `(dx_mm, dz_mm)` from
/// the feet. Depth comes from the obstacle record; this probe measures
/// height, drop and gap only.
pub fn sense_ahead(
    walker: &Walker,
    dx_mm: i32,
    dz_mm: i32,
    world: &StepWorld<'_>,
    cfg: &Config,
) -> AheadSense {
    let tx = narrow(walker.pos_mm[0], i64::from(dx_mm));
    let tz = narrow(walker.pos_mm[2], i64::from(dz_mm));
    let (support, _) = support_at(tx, tz, walker.pos_mm[1], world, cfg);
    let mut wall: Option<i32> = None;
    for box_ in world.blocks {
        if tx >= box_.min_mm[0] - RADIUS_MM
            && tx <= box_.max_mm[0] + RADIUS_MM
            && tz >= box_.min_mm[2] - RADIUS_MM
            && tz <= box_.max_mm[2] + RADIUS_MM
            && box_.max_mm[1] > walker.pos_mm[1] + cfg.step_height_mm
            && wall.is_none_or(|w| box_.max_mm[1] < w)
        {
            wall = Some(box_.max_mm[1]);
        }
    }
    let len =
        u64::try_from(i64::from(dx_mm) * i64::from(dx_mm) + i64::from(dz_mm) * i64::from(dz_mm))
            .unwrap_or(u64::MAX);
    let len = len.isqrt();
    let mut gap_mm = SENSE_RANGE_MM;
    let march = i64::try_from(len).unwrap_or(i64::MAX);
    if len > 0 {
        let steps = SENSE_RANGE_MM / SENSE_STRIDE_MM;
        for stride in 1..=steps {
            let reach = stride * SENSE_STRIDE_MM;
            let px = narrow(
                walker.pos_mm[0],
                i64::from(dx_mm) * i64::from(reach) / march,
            );
            let pz = narrow(
                walker.pos_mm[2],
                i64::from(dz_mm) * i64::from(reach) / march,
            );
            let (found, _) = support_at(px, pz, walker.pos_mm[1], world, cfg);
            if found.is_some() {
                gap_mm = reach;
                break;
            }
        }
    } else {
        gap_mm = 0;
    }
    AheadSense {
        support_mm: support,
        wall_mm: wall,
        gap_mm,
    }
}

/// Carry a grounded rider by its platform delta; names the ridden platform and the travel it
/// applied this tick.
fn ride_platform(walker: &mut Walker, world: &StepWorld<'_>) -> Option<(u32, [i32; 3])> {
    if !walker.grounded {
        return None;
    }
    let id = walker.ground_platform?;
    for platform in world.platforms {
        if platform.id == id {
            walker.pos_mm[0] = narrow(walker.pos_mm[0], i64::from(platform.delta_mm[0]));
            walker.pos_mm[1] = narrow(walker.pos_mm[1], i64::from(platform.delta_mm[1]));
            walker.pos_mm[2] = narrow(walker.pos_mm[2], i64::from(platform.delta_mm[2]));
            return Some((id, platform.delta_mm));
        }
    }
    None
}

/// One-shot step-off carry: a rider that left its deck this tick keeps the deck's travel as
/// velocity, so the flight keeps the platform's motion. Integer mm/s, saturating; a rider on a
/// deck that has stopped inherits zero and simply drops.
fn inherit_ride(walker: &mut Walker, delta_mm: [i32; 3]) {
    for (lane, delta) in walker.vel_mm_s.iter_mut().zip(delta_mm) {
        *lane = narrow(*lane, i64::from(delta) * i64::from(TICKS_PER_S));
    }
}

/// Steer horizontal velocity toward the clamped wish.
fn steer(walker: &mut Walker, wish_mm_s: [i32; 2], cfg: &Config) {
    let clamp_wish = cfg.max_wish_mm_s.max(0);
    let wish = [
        wish_mm_s[0].clamp(-clamp_wish, clamp_wish),
        wish_mm_s[1].clamp(-clamp_wish, clamp_wish),
    ];
    let accel = if walker.grounded {
        cfg.grip_accel_mm_s2()
    } else {
        cfg.accel_air_mm_s2.max(0)
    };
    let dv_max = accel / TICKS_PER_S;
    for (axis, want) in wish.into_iter().enumerate() {
        let lane = if axis == 0 {
            &mut walker.vel_mm_s[0]
        } else {
            &mut walker.vel_mm_s[2]
        };
        let delta = (want - *lane).clamp(-dv_max, dv_max);
        *lane += delta;
    }
}

/// Fire a buffered jump, then pull down with clamped gravity.
fn jump_and_gravity(walker: &mut Walker, jump_pressed: bool, cfg: &Config) {
    if jump_pressed {
        walker.buffer_ticks = BUFFER_TICKS;
    }
    if walker.buffer_ticks > 0 && (walker.grounded || walker.coyote_ticks > 0) {
        walker.vel_mm_s[1] = cfg.jump_v_mm_s.max(0);
        walker.grounded = false;
        walker.ground_platform = None;
        walker.buffer_ticks = 0;
        walker.coyote_ticks = 0;
    }
    walker.vel_mm_s[1] -= GRAVITY_MM_S2 / TICKS_PER_S;
    if walker.vel_mm_s[1] < MAX_FALL_MM_S {
        walker.vel_mm_s[1] = MAX_FALL_MM_S;
    }
}

/// Slide along both horizontal axes with autostep; reports steps and walls.
fn move_horizontal(
    walker: &mut Walker,
    world: &StepWorld<'_>,
    cfg: &Config,
) -> (bool, Option<[i32; 2]>) {
    let mut stepped = false;
    let mut wall_normal: Option<[i32; 2]> = None;
    for axis in 0..2 {
        let lane = if axis == 0 { 0 } else { 2 };
        let delta = travel(walker.vel_mm_s[lane], &mut walker.carry[lane]);
        if delta == 0 {
            continue;
        }
        let (nx, nz) = if axis == 0 {
            (narrow(walker.pos_mm[0], delta), walker.pos_mm[2])
        } else {
            (walker.pos_mm[0], narrow(walker.pos_mm[2], delta))
        };
        let mut blocked = false;
        for box_ in world.blocks {
            if column_hits_box(nx, nz, walker.pos_mm[1], box_) {
                blocked = true;
                break;
            }
        }
        if !blocked {
            walker.pos_mm[0] = nx;
            walker.pos_mm[2] = nz;
            continue;
        }
        let (lift, _) = support_at(nx, nz, walker.pos_mm[1], world, cfg);
        let rise = lift.map(|top| top - walker.pos_mm[1]);
        if rise.is_some_and(|r| r > 0 && r <= cfg.step_height_mm)
            && lift.is_some_and(|top| column_free(nx, nz, top, world))
        {
            walker.pos_mm[0] = nx;
            walker.pos_mm[2] = nz;
            walker.pos_mm[1] = lift.unwrap_or(walker.pos_mm[1]);
            stepped = true;
        } else {
            let sign = if delta > 0 { -1 } else { 1 };
            wall_normal = if axis == 0 {
                Some([sign, 0])
            } else {
                Some([0, sign])
            };
            walker.vel_mm_s[lane] = 0;
            walker.carry[lane] = 0;
        }
    }
    (stepped, wall_normal)
}

/// Land, snap, fall or slip downhill; reports steep-slope slips. Open air
/// past every support simply keeps falling.
fn resolve_vertical(walker: &mut Walker, world: &StepWorld<'_>, cfg: &Config) -> bool {
    // The remainder commits only where the walker really travels to `y_new`; landing, snapping,
    // a ceiling and a slip stop the lane and clear it.
    let mut carry_y = walker.carry[1];
    let y_new = narrow(walker.pos_mm[1], travel(walker.vel_mm_s[1], &mut carry_y));
    walker.carry[1] = 0;
    let mut slipped = false;
    if walker.vel_mm_s[1] > 0 {
        let mut ceiling = i32::MAX;
        for box_ in world.blocks {
            if walker.pos_mm[0] >= box_.min_mm[0] - RADIUS_MM
                && walker.pos_mm[0] <= box_.max_mm[0] + RADIUS_MM
                && walker.pos_mm[2] >= box_.min_mm[2] - RADIUS_MM
                && walker.pos_mm[2] <= box_.max_mm[2] + RADIUS_MM
                && box_.min_mm[1] >= walker.pos_mm[1] + HEIGHT_MM
                && box_.min_mm[1] < ceiling
            {
                ceiling = box_.min_mm[1];
            }
        }
        if ceiling < i32::MAX && y_new + HEIGHT_MM > ceiling {
            walker.pos_mm[1] = ceiling - HEIGHT_MM;
            walker.vel_mm_s[1] = 0;
        } else {
            walker.pos_mm[1] = y_new;
            walker.carry[1] = carry_y;
        }
        walker.grounded = false;
        walker.ground_platform = None;
    } else {
        let (support, ride) = support_at(
            walker.pos_mm[0],
            walker.pos_mm[2],
            walker.pos_mm[1],
            world,
            cfg,
        );
        match support {
            Some(top) if y_new <= top => {
                walker.pos_mm[1] = top;
                walker.vel_mm_s[1] = 0;
                walker.grounded = true;
                walker.ground_platform = ride;
                walker.coyote_ticks = COYOTE_TICKS;
            }
            Some(top)
                if walker.grounded
                    && top < walker.pos_mm[1]
                    && walker.pos_mm[1] - top <= cfg.snap_mm =>
            {
                walker.pos_mm[1] = top;
                walker.vel_mm_s[1] = 0;
                walker.ground_platform = ride;
            }
            _ => {
                let fell_from = walker.pos_mm[1];
                walker.pos_mm[1] = y_new;
                walker.carry[1] = carry_y;
                walker.grounded = false;
                walker.ground_platform = None;
                if let Some((surface, downhill)) =
                    steep_surface_at(walker.pos_mm[0], walker.pos_mm[2], world, cfg)
                {
                    let crossed = fell_from >= surface && y_new <= surface;
                    let resting = y_new <= surface && y_new >= surface - cfg.snap_mm;
                    if crossed || resting {
                        let slide = SLIP_MM_S / TICKS_PER_S;
                        let sx = narrow(walker.pos_mm[0], i64::from(downhill * slide));
                        walker.pos_mm[0] = sx;
                        walker.pos_mm[1] = steep_surface_at(sx, walker.pos_mm[2], world, cfg)
                            .map_or(surface, |(top, _)| top);
                        walker.vel_mm_s[1] = 0;
                        walker.carry[1] = 0;
                        walker.grounded = true;
                        walker.coyote_ticks = COYOTE_TICKS;
                        slipped = true;
                    }
                }
            }
        }
    }
    slipped
}

/// Advance one 1/60 s tick: platform ride, accel, jump, gravity, slide with
/// autostep, snap and slope slip. Allocation-free; world slices are read in
/// the caller's stable order.
pub fn step_walker(
    walker: &mut Walker,
    input: &TickInput,
    world: &StepWorld<'_>,
    cfg: &Config,
) -> Resolved {
    let from = walker.pos_mm;
    let rode = ride_platform(walker, world);
    steer(walker, input.wish_mm_s, cfg);
    jump_and_gravity(walker, input.jump_pressed, cfg);
    let (stepped, wall_normal) = move_horizontal(walker, world, cfg);
    let slipped = resolve_vertical(walker, world, cfg);
    if let Some((_, ride_mm)) = rode {
        if !walker.grounded {
            inherit_ride(walker, ride_mm);
        }
    }
    walker.buffer_ticks = walker.buffer_ticks.saturating_sub(1);
    if !walker.grounded {
        walker.coyote_ticks = walker.coyote_ticks.saturating_sub(1);
    }
    Resolved {
        displacement_mm: [walker.pos_mm[0] - from[0], walker.pos_mm[2] - from[2]],
        vertical_mm: walker.pos_mm[1] - from[1],
        grounded: walker.grounded,
        wall_normal,
        stepped,
        slipped,
        rode_platform: rode.map(|(id, _)| id),
    }
}

/// Push two overlapping fighters apart; positional only, split evenly in a
/// canonical id order so arg order never decides the result. No-op at rest
/// distance; the degenerate coincident case splits along x by lower id.
pub fn separate_pair(a: &mut Walker, b: &mut Walker) {
    let need = 2 * RADIUS_MM;
    let dx = i64::from(b.pos_mm[0]) - i64::from(a.pos_mm[0]);
    let dz = i64::from(b.pos_mm[2]) - i64::from(a.pos_mm[2]);
    let dist2 = u64::try_from(dx * dx + dz * dz).unwrap_or(u64::MAX);
    let need64 = u64::try_from(need).unwrap_or(0);
    let need2 = need64 * need64;
    if dist2 >= need2 {
        return;
    }
    if dist2 == 0 {
        let (sign_a, sign_b) = if a.id < b.id { (-1, 1) } else { (1, -1) };
        let half = need / 2;
        a.pos_mm[0] = narrow(a.pos_mm[0], i64::from(sign_a * half));
        b.pos_mm[0] = narrow(b.pos_mm[0], i64::from(sign_b * half));
        return;
    }
    let dist = i64::try_from(dist2.isqrt()).unwrap_or(i64::MAX);
    let overlap = i64::from(need) - dist;
    let step = (overlap / 2).max(1);
    a.pos_mm[0] = narrow(a.pos_mm[0], -dx * step / dist);
    a.pos_mm[2] = narrow(a.pos_mm[2], -dz * step / dist);
    b.pos_mm[0] = narrow(b.pos_mm[0], dx * step / dist);
    b.pos_mm[2] = narrow(b.pos_mm[2], dz * step / dist);
}

/// FEEL10 contact push: two bodies in contact exchange equal and opposite impulses along
/// the contact normal, so close contact pushes back and not only apart.
///
/// `pa_mm` and `pb_mm` are planar centres in millimetres, `va` and `vb` planar velocities in
/// the caller's unit (the same unit for both, the unit it steps by). Equal masses: the
/// closing speed along the normal is halved into each body, a restitution of zero, so a
/// bounce never appears. Returns the velocity change `b` takes; `a` takes the exact negation,
/// so the pair's total planar momentum never moves. `None` when the pair is apart, or not
/// closing (a resting pair, or a pair moving apart, gives no push). Coincident centres take
/// the normal `+x` from `a` to `b`: callers pass the lower slot as `a`. Integer only; `/`
/// truncates toward zero, so the result is mirror-symmetric.
pub fn push_exchange(
    pa_mm: [i32; 2],
    pb_mm: [i32; 2],
    va: [i32; 2],
    vb: [i32; 2],
) -> Option<[i32; 2]> {
    let need = i64::from(2 * RADIUS_MM);
    let dx = i64::from(pb_mm[0]) - i64::from(pa_mm[0]);
    let dz = i64::from(pb_mm[1]) - i64::from(pa_mm[1]);
    let dist2 = dx * dx + dz * dz;
    if dist2 >= need * need {
        return None;
    }
    let (dx, dz, dist2) = if dist2 == 0 {
        (1, 0, 1)
    } else {
        (dx, dz, dist2)
    };
    // Closing rate along the normal times the centre distance: a plain integer, so the
    // impulse below needs no square root.
    let rel_x = i64::from(vb[0]) - i64::from(va[0]);
    let rel_z = i64::from(vb[1]) - i64::from(va[1]);
    let closing = rel_x * dx + rel_z * dz;
    if closing >= 0 {
        return None;
    }
    // b takes -closing / (2 dist) along the unit normal, which is -closing * d / (2 dist2).
    let den = 2 * dist2;
    Some([
        narrow(0, -closing * dx / den),
        narrow(0, -closing * dz / den),
    ])
}

/// Crowd shove: add a capped impulse along `dir_mm_s`; a zero direction or
/// a non-positive strength leaves the walker untouched.
pub fn apply_shove(walker: &mut Walker, dir_mm: [i32; 2], strength_mm_s: i32) {
    if strength_mm_s <= 0 {
        return;
    }
    let len2 = u64::try_from(
        i64::from(dir_mm[0]) * i64::from(dir_mm[0]) + i64::from(dir_mm[1]) * i64::from(dir_mm[1]),
    )
    .unwrap_or(u64::MAX);
    if len2 == 0 {
        return;
    }
    let push = strength_mm_s.min(MAX_SHOVE_MM_S);
    let len = i64::try_from(len2.isqrt()).unwrap_or(i64::MAX);
    walker.vel_mm_s[0] += narrow(0, i64::from(dir_mm[0]) * i64::from(push) / len);
    walker.vel_mm_s[2] += narrow(0, i64::from(dir_mm[1]) * i64::from(push) / len);
}

/// FNV-1a64 over the walker lanes; identical walkers hash identically.
pub fn state_hash(walker: &Walker) -> u64 {
    let mut h = Fnv64::new();
    h.write_u32(walker.id);
    for lane in walker.pos_mm {
        h.write_i32(lane);
    }
    for lane in walker.vel_mm_s {
        h.write_i32(lane);
    }
    h.write_bool(walker.grounded);
    h.write_u32(walker.ground_platform.unwrap_or(u32::MAX));
    h.write_u32(walker.coyote_ticks);
    h.write_u32(walker.buffer_ticks);
    for lane in walker.carry {
        h.write_i32(lane);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe;
    use engine_core::laws;

    /// One tick of a two-walker contact: the push exchange, then the positional
    /// separation, then the walkers carry their velocity.
    fn push_step(pair: &mut (Walker, Walker)) {
        let (a, b) = pair;
        if let Some(d) = push_exchange(
            [a.pos_mm[0], a.pos_mm[2]],
            [b.pos_mm[0], b.pos_mm[2]],
            [a.vel_mm_s[0], a.vel_mm_s[2]],
            [b.vel_mm_s[0], b.vel_mm_s[2]],
        ) {
            b.vel_mm_s[0] += d[0];
            b.vel_mm_s[2] += d[1];
            a.vel_mm_s[0] -= d[0];
            a.vel_mm_s[2] -= d[1];
        }
        separate_pair(a, b);
        a.pos_mm[0] += a.vel_mm_s[0] / TICKS_PER_S;
        a.pos_mm[2] += a.vel_mm_s[2] / TICKS_PER_S;
        b.pos_mm[0] += b.vel_mm_s[0] / TICKS_PER_S;
        b.pos_mm[2] += b.vel_mm_s[2] / TICKS_PER_S;
    }

    /// Planar momentum of the pair, squared, so one number carries both lanes.
    fn pair_momentum_sq(pair: &(Walker, Walker)) -> f64 {
        let x = i64::from(pair.0.vel_mm_s[0]) + i64::from(pair.1.vel_mm_s[0]);
        let z = i64::from(pair.0.vel_mm_s[2]) + i64::from(pair.1.vel_mm_s[2]);
        (x * x + z * z) as f64
    }

    /// True when `b` is `a` seen in a mirror across the z axis (x flips sign).
    fn is_x_mirror(a: &(Walker, Walker), b: &(Walker, Walker)) -> bool {
        [(&a.0, &b.0), (&a.1, &b.1)].iter().all(|(p, q)| {
            p.pos_mm[0] == -q.pos_mm[0]
                && p.pos_mm[2] == q.pos_mm[2]
                && p.vel_mm_s[0] == -q.vel_mm_s[0]
                && p.vel_mm_s[2] == q.vel_mm_s[2]
        })
    }

    #[test]
    fn a_contact_push_conserves_momentum() {
        // a comes in at 800 mm/s, b stands 500 mm off: they overlap, b takes a share of a's
        // speed and a loses exactly that share. The pair's momentum never moves.
        let start = (
            Walker {
                vel_mm_s: [800, 0, 0],
                ..Walker::spawn(0, [0, 0, 0])
            },
            Walker::spawn(1, [500, 0, 0]),
        );
        let mut once = start;
        push_step(&mut once);
        assert_eq!((once.0.vel_mm_s[0], once.1.vel_mm_s[0]), (400, 400));
        laws::conserved(start, 300, push_step, pair_momentum_sq, 0.0).unwrap();
    }

    #[test]
    fn a_resting_pair_stays_at_rest_under_the_push() {
        // Touching at exactly two radii, standing still: no push, no move, bit-identical.
        let touching = (
            Walker::spawn(0, [0, 0, 0]),
            Walker::spawn(1, [2 * RADIUS_MM, 0, 0]),
        );
        laws::rest_stays_rest(touching, 600, push_step).unwrap();
        // Overlapping but standing still: nothing is closing, so no push at all.
        assert_eq!(push_exchange([0, 0], [RADIUS_MM, 0], [0, 0], [0, 0]), None);
        // Moving apart gives no push either.
        assert_eq!(push_exchange([0, 0], [RADIUS_MM, 0], [-5, 0], [5, 0]), None);
    }

    #[test]
    fn mirrored_contact_pushes_mirror() {
        // The east pair, then its x-mirror image: the push must mirror exactly, z lanes included.
        let east = (
            Walker {
                vel_mm_s: [800, 0, 300],
                ..Walker::spawn(0, [0, 0, 0])
            },
            Walker::spawn(1, [500, 0, -200]),
        );
        let west = (
            Walker {
                vel_mm_s: [-800, 0, 300],
                ..Walker::spawn(0, [0, 0, 0])
            },
            Walker::spawn(1, [-500, 0, -200]),
        );
        assert!(is_x_mirror(&east, &west));
        laws::mirror(east, west, 300, |p, _| push_step(p), is_x_mirror).unwrap();
    }

    fn slab() -> Vec<Block> {
        vec![Block::new([-50_000, -1_000, -50_000], [50_000, 0, 50_000])]
    }

    fn world_with<'a>(
        blocks: &'a [Block],
        ramps: &'a [Ramp],
        platforms: &'a [Platform],
    ) -> StepWorld<'a> {
        StepWorld {
            blocks,
            ramps,
            platforms,
        }
    }

    fn walk_forward(world: &StepWorld<'_>, cfg: &Config, start: [i32; 3], ticks: u32) -> Walker {
        let mut w = Walker::spawn(7, start);
        let input = TickInput {
            wish_mm_s: [3_000, 0],
            jump_pressed: false,
        };
        for _ in 0..ticks {
            step_walker(&mut w, &input, world, cfg);
        }
        w
    }

    #[test]
    fn step_climbs_stairs() {
        let cfg = Config::default();
        let mut blocks = slab();
        blocks.push(Block::new([2_000, 0, -2_000], [3_000, 150, 2_000]));
        blocks.push(Block::new([3_000, 0, -2_000], [4_000, 300, 2_000]));
        blocks.push(Block::new([4_000, 0, -2_000], [12_000, 450, 2_000]));
        let world = world_with(&blocks, &[], &[]);
        let w = walk_forward(&world, &cfg, [0, 0, 0], 160);
        assert!(w.pos_mm[0] > 5_000, "walker stuck at {:?}", w.pos_mm);
        assert_eq!(w.pos_mm[1], 450);
        assert!(w.grounded);
    }

    #[test]
    fn material_table_default_ground_is_todays_constants() {
        // The default ground is the table's GROUND row, and that row reproduces the legacy
        // constants exactly: nothing a hashed run reads moved.
        let legacy = Config {
            step_height_mm: STEP_HEIGHT_MM,
            snap_mm: SNAP_MM,
            max_slope_grade_q: MAX_SLOPE_GRADE_Q,
            accel_ground_mm_s2: ACCEL_GROUND_MM_S2,
            accel_air_mm_s2: ACCEL_AIR_MM_S2,
            max_wish_mm_s: MAX_WISH_MM_S,
            jump_v_mm_s: JUMP_V_MM_S,
            friction_pm: 800,
        };
        assert_eq!(Config::default(), legacy);
        assert_eq!(Config::default().grip_accel_mm_s2(), ACCEL_GROUND_MM_S2);
        assert_eq!(Config::on(GROUND), legacy);
    }

    #[test]
    fn material_table_grip_sets_how_fast_a_walker_gets_going() {
        let blocks = slab();
        let world = world_with(&blocks, &[], &[]);
        let first_tick_speed = |ground: Material| {
            let cfg = Config::on(ground);
            assert_eq!(cfg.friction_pm, i32::from(ground.props().friction_pm));
            // One idle tick lands the spawned walker; the next tick is a grounded one.
            let mut w = Walker::spawn(7, [0, 0, 0]);
            step_walker(&mut w, &TickInput::idle(), &world, &cfg);
            assert!(w.grounded, "{ground:?}");
            let input = TickInput {
                wish_mm_s: [3_000, 0],
                jump_pressed: false,
            };
            step_walker(&mut w, &input, &world, &cfg);
            w.vel_mm_s[0]
        };
        // One tick of the base 60000 mm/s^2 is 1000 mm/s on the 800 row; the others scale.
        assert_eq!(first_tick_speed(Material::Stone), 1_000);
        assert_eq!(first_tick_speed(Material::Metal), 500);
        assert_eq!(first_tick_speed(Material::Sand), 1_250);
        let mut by_grip: Vec<(i32, i32)> = Material::ALL
            .iter()
            .map(|m| (i32::from(m.props().friction_pm), first_tick_speed(*m)))
            .collect();
        by_grip.sort_unstable();
        assert!(by_grip.windows(2).all(|p| p[0].1 <= p[1].1), "{by_grip:?}");
    }

    #[test]
    fn material_table_soft_ground_gives_a_lower_step() {
        let mut blocks = slab();
        blocks.push(Block::new([2_000, 0, -2_000], [12_000, 450, 2_000]));
        let world = world_with(&blocks, &[], &[]);
        let on_stone = walk_forward(&world, &Config::on(Material::Stone), [0, 0, 0], 160);
        assert_eq!(on_stone.pos_mm[1], 450, "stone steps a 450 mm rise");
        let on_sand = walk_forward(&world, &Config::on(Material::Sand), [0, 0, 0], 160);
        assert_eq!(on_sand.pos_mm[1], 0, "sand gives way under a 450 mm rise");
        assert!(on_sand.pos_mm[0] < 2_000, "{:?}", on_sand.pos_mm);
    }

    #[test]
    fn slope_walkable_climbs_steep_slips() {
        let cfg = Config::default();
        let blocks = slab();
        let gentle = Ramp {
            x0_mm: 2_000,
            x1_mm: 26_000,
            z0_mm: -2_000,
            z1_mm: 2_000,
            y_base_mm: 0,
            rise_mm: 2_000,
        };
        let gentle_ramps = [gentle];
        let world = world_with(&blocks, &gentle_ramps, &[]);
        let w = walk_forward(&world, &cfg, [0, 0, 0], 500);
        assert!(w.pos_mm[0] > 20_000, "gentle ramp not climbed: {w:?}");
        assert!(
            (1_500..=2_100).contains(&w.pos_mm[1]),
            "ramp top not reached: {w:?}"
        );

        let steep = Ramp {
            x0_mm: 2_000,
            x1_mm: 6_000,
            z0_mm: -2_000,
            z1_mm: 2_000,
            y_base_mm: 0,
            rise_mm: 6_000,
        };
        let steep_ramps = [steep];
        let world = world_with(&blocks, &steep_ramps, &[]);
        let mut w = Walker::spawn(9, [3_000, 2_000, 0]);
        let input = TickInput::idle();
        let mut slipped = false;
        for _ in 0..240 {
            let r = step_walker(&mut w, &input, &world, &cfg);
            slipped |= r.slipped;
        }
        assert!(slipped, "steep ramp never slipped: {w:?}");
        assert!(w.grounded, "slipped walker left ungrounded: {w:?}");
    }

    #[test]
    fn platform_ride_keeps_station() {
        let cfg = Config::default();
        let blocks = slab();
        let mut platform = Platform {
            id: 3,
            min_mm: [-1_000, -200, -1_000],
            max_mm: [1_000, 0, 1_000],
            delta_mm: [5, 0, 0],
        };
        let mut w = Walker::spawn(5, [0, 0, 0]);
        let input = TickInput::idle();
        {
            let world = world_with(&blocks, &[], core::slice::from_ref(&platform));
            step_walker(&mut w, &input, &world, &cfg);
        }
        platform.min_mm[0] += platform.delta_mm[0];
        platform.max_mm[0] += platform.delta_mm[0];
        assert!(w.grounded);
        assert_eq!(w.ground_platform, Some(3));
        let offset = w.pos_mm[0] - platform.min_mm[0];
        for _ in 0..600 {
            {
                let world = world_with(&blocks, &[], core::slice::from_ref(&platform));
                let r = step_walker(&mut w, &input, &world, &cfg);
                assert_eq!(r.rode_platform, Some(3));
            }
            platform.min_mm[0] += platform.delta_mm[0];
            platform.max_mm[0] += platform.delta_mm[0];
        }
        assert_eq!(w.pos_mm[0] - platform.min_mm[0], offset);
        assert_eq!(w.pos_mm[1], 0);
        assert!(w.grounded);
    }

    /// Independent f64 prediction of a step-off flight: air steer toward the wish, gravity
    /// per tick, then travel. Returns the landing x and the flight tick. It reads the
    /// documented tick law, never the stepper, so a drift on either side shows up.
    fn ballistic_landing(cfg: &Config, launch: &Walker, wish_x_mm_s: i32) -> (f64, u32) {
        let dt = 1.0 / f64::from(TICKS_PER_S);
        let steer_max = f64::from(cfg.accel_air_mm_s2.max(0) / TICKS_PER_S);
        let gravity = f64::from(GRAVITY_MM_S2 / TICKS_PER_S);
        let wish = wish_x_mm_s.clamp(-cfg.max_wish_mm_s, cfg.max_wish_mm_s);
        let mut x = f64::from(launch.pos_mm[0]);
        let mut y = f64::from(launch.pos_mm[1]);
        let mut vx = f64::from(launch.vel_mm_s[0]);
        let mut vy = f64::from(launch.vel_mm_s[1]);
        for tick in 1..=600u32 {
            vx += (f64::from(wish) - vx).clamp(-steer_max, steer_max);
            vy -= gravity;
            x += vx * dt;
            y += vy * dt;
            if y <= 0.0 {
                return (x, tick);
            }
        }
        (x, 0)
    }

    #[test]
    fn platform_ride_matrix() {
        let cfg = Config::default();
        let blocks = slab();

        // Ride, then stop the deck: the rider keeps station through both, and a stopped
        // deck never throws it.
        let mut stop_hashes = [0u64; 2];
        for (rep, hash) in stop_hashes.iter_mut().enumerate() {
            let mut platform = Platform {
                id: 3,
                min_mm: [-1_000, -200, -1_000],
                max_mm: [1_000, 0, 1_000],
                delta_mm: [5, 0, 0],
            };
            let mut w = Walker::spawn(5, [0, 0, 0]);
            let input = TickInput::idle();
            {
                let world = world_with(&blocks, &[], core::slice::from_ref(&platform));
                step_walker(&mut w, &input, &world, &cfg);
            }
            platform.min_mm[0] += platform.delta_mm[0];
            platform.max_mm[0] += platform.delta_mm[0];
            assert!(w.grounded && w.ground_platform == Some(3), "rep {rep}");
            let station = w.pos_mm[0] - platform.min_mm[0];
            for tick in 0..660 {
                platform.delta_mm = if tick < 600 { [5, 0, 0] } else { [0, 0, 0] };
                let world = world_with(&blocks, &[], core::slice::from_ref(&platform));
                let r = step_walker(&mut w, &input, &world, &cfg);
                assert_eq!(r.rode_platform, Some(3), "rep {rep} tick {tick}");
                platform.min_mm[0] += platform.delta_mm[0];
                platform.max_mm[0] += platform.delta_mm[0];
                assert_eq!(
                    w.pos_mm[0] - platform.min_mm[0],
                    station,
                    "rep {rep} tick {tick} left station"
                );
            }
            assert!(w.grounded);
            assert_eq!(w.vel_mm_s, [0, 0, 0], "a stopped deck must not throw");
            *hash = state_hash(&w);
        }
        assert_eq!(stop_hashes[0], stop_hashes[1], "platform stop x2");

        // Step-off: a rider walks off a deck moving 1200 mm/s toward it. It inherits the
        // deck speed once, then flies and lands where the f64 prediction says. The same
        // flight without the carry must fall more than 1 cm short.
        let delta_mm = [20, 0, 0];
        let deck_top = 1_500;
        let wish = 3_000;
        let input = TickInput {
            wish_mm_s: [wish, 0],
            jump_pressed: false,
        };
        let mut off_hashes = [0u64; 2];
        for (rep, hash) in off_hashes.iter_mut().enumerate() {
            let mut platform = Platform {
                id: 7,
                min_mm: [-2_000, deck_top - 200, -1_000],
                max_mm: [2_000, deck_top, 1_000],
                delta_mm,
            };
            let mut w = Walker::spawn(9, [0, deck_top, 0]);
            w.grounded = true;
            let mut launch: Option<Walker> = None;
            let mut landed: Option<(u32, i32)> = None;
            let mut flight = 0u32;
            for _ in 0..600 {
                let world = world_with(&blocks, &[], core::slice::from_ref(&platform));
                let r = step_walker(&mut w, &input, &world, &cfg);
                platform.min_mm[0] += delta_mm[0];
                platform.max_mm[0] += delta_mm[0];
                if launch.is_none() {
                    if !w.grounded {
                        assert_eq!(r.rode_platform, Some(7), "rep {rep} left the deck");
                        assert_eq!(
                            w.vel_mm_s[0],
                            wish + delta_mm[0] * TICKS_PER_S,
                            "rep {rep} step-off must carry the deck speed"
                        );
                        launch = Some(w);
                    }
                } else {
                    flight += 1;
                    if w.grounded {
                        landed = Some((flight, w.pos_mm[0]));
                        break;
                    }
                }
            }
            let launch = launch.expect("rider never left the deck");
            let (landed_ticks, landed_x) = landed.expect("rider never landed");
            let (want_x, want_ticks) = ballistic_landing(&cfg, &launch, wish);
            assert_eq!(landed_ticks, want_ticks, "rep {rep} landing tick");
            // Positions are integer millimetres; the row's bar is one centimetre.
            let one_cm_mm = 10.0;
            let miss = (f64::from(landed_x) - want_x).abs();
            assert!(
                miss <= one_cm_mm,
                "rep {rep} lands {miss} mm from the ballistic target"
            );
            let mut bare = launch;
            bare.vel_mm_s[0] -= delta_mm[0] * TICKS_PER_S;
            let (bare_x, _) = ballistic_landing(&cfg, &bare, wish);
            assert!(
                f64::from(landed_x) - bare_x > one_cm_mm,
                "rep {rep} carry moved the landing {} mm, under the 1 cm bar",
                f64::from(landed_x) - bare_x
            );
            assert_eq!(w.pos_mm[1], 0, "rep {rep} landed on the slab");
            *hash = state_hash(&w);
        }
        assert_eq!(off_hashes[0], off_hashes[1], "step-off x2");
    }

    #[test]
    fn ledge_table_matches_tv1() {
        assert_eq!(classify(0, 0, 0, 0, false), Ok(LedgeAction::None));
        assert_eq!(classify(199, 0, 0, 0, false), Ok(LedgeAction::None));
        assert_eq!(classify(600, 400, 0, 2_000, false), Ok(LedgeAction::Hurdle));
        assert_eq!(classify(600, 400, 0, 500, false), Ok(LedgeAction::Mantle));
        assert_eq!(classify(600, 900, 0, 5_000, false), Ok(LedgeAction::Mantle));
        assert_eq!(classify(1_000, 900, 0, 0, false), Ok(LedgeAction::Vault));
        assert_eq!(classify(1_000, 1_500, 0, 0, false), Ok(LedgeAction::Mantle));
        assert_eq!(classify(1_500, 3_000, 0, 0, false), Ok(LedgeAction::Mantle));
        assert_eq!(classify(2_200, 0, 0, 0, true), Ok(LedgeAction::Climb));
        assert_eq!(classify(2_200, 0, 0, 0, false), Err(ProbeError::NoHold));
        assert_eq!(classify(-500, 0, 0, 0, false), Ok(LedgeAction::Drop));
        assert_eq!(
            classify(0, 0, 2_600, 0, false),
            Err(ProbeError::GapTooWide {
                gap_mm: 2_600,
                max_mm: 2_500
            })
        );
        assert_eq!(
            classify(3_000, 0, 0, 0, true),
            Err(ProbeError::BadProbe("height"))
        );
        assert_eq!(
            classify(-3_000, 0, 0, 0, false),
            Err(ProbeError::BadProbe("drop"))
        );
        assert_eq!(duration_ticks(LedgeAction::Climb), 60);
        assert_eq!(duration_ticks(LedgeAction::None), 0);
    }

    #[test]
    fn ledge_sense_feeds_classify() {
        let cfg = Config::default();
        let mut blocks = slab();
        blocks.push(Block::new([1_500, 0, -500], [2_500, 1_000, 500]));
        let world = world_with(&blocks, &[], &[]);
        let w = Walker::spawn(11, [0, 0, 0]);
        let sense = sense_ahead(&w, 2_000, 0, &world, &cfg);
        assert_eq!(sense.wall_mm, Some(1_000));
        let action = classify(1_000, 900, 0, 0, false);
        assert_eq!(action, Ok(LedgeAction::Vault));
    }

    #[test]
    fn push_and_shove_separate() {
        let mut a = Walker::spawn(1, [0, 0, 0]);
        let mut b = Walker::spawn(2, [200, 0, 0]);
        separate_pair(&mut a, &mut b);
        let gap = b.pos_mm[0] - a.pos_mm[0];
        assert!(gap >= 2 * RADIUS_MM, "pair still overlaps: {gap}");
        separate_pair(&mut b, &mut a);
        let gap2 = (b.pos_mm[0] - a.pos_mm[0]).abs();
        assert!(gap2 >= 2 * RADIUS_MM, "swapped args differ: {gap2}");

        let mut w = Walker::spawn(3, [0, 0, 0]);
        apply_shove(&mut w, [0, 0], 5_000);
        assert_eq!(w.vel_mm_s, [0, 0, 0]);
        apply_shove(&mut w, [1_000, 0], 0);
        assert_eq!(w.vel_mm_s, [0, 0, 0]);
        apply_shove(&mut w, [3_000, 4_000], 5_000);
        assert_eq!(w.vel_mm_s[0], 3_000);
        assert_eq!(w.vel_mm_s[2], 4_000);
        apply_shove(&mut w, [1_000, 0], 99_000);
        assert!(w.vel_mm_s[0] <= MAX_WISH_MM_S + MAX_SHOVE_MM_S);
    }

    #[test]
    fn replay_exact_x2_with_snapshot() {
        let cfg = Config::default();
        let mut blocks = slab();
        blocks.push(Block::new([2_000, 0, -2_000], [3_000, 150, 2_000]));
        let gentle = Ramp {
            x0_mm: 5_000,
            x1_mm: 9_000,
            z0_mm: -2_000,
            z1_mm: 2_000,
            y_base_mm: 0,
            rise_mm: 1_000,
        };
        let platform = Platform {
            id: 8,
            min_mm: [10_000, -200, -1_000],
            max_mm: [12_000, 0, 1_000],
            delta_mm: [0, 0, 0],
        };
        let run = || {
            let world = world_with(
                &blocks,
                core::slice::from_ref(&gentle),
                core::slice::from_ref(&platform),
            );
            let mut w = Walker::spawn(21, [0, 0, 0]);
            let mut checkpoints = [0_u64; 10];
            for tick in 0..600 {
                let input = TickInput {
                    wish_mm_s: [3_000, 0],
                    jump_pressed: tick == 60,
                };
                step_walker(&mut w, &input, &world, &cfg);
                if tick % 60 == 59 {
                    let slot = usize::try_from(tick / 60).expect("tick under 600 fits usize");
                    checkpoints[slot] = state_hash(&w);
                }
            }
            (w, checkpoints)
        };
        let (first, first_hashes) = run();
        let (second, second_hashes) = run();
        assert_eq!(first_hashes, second_hashes);
        assert_eq!(state_hash(&first), state_hash(&second));

        let world = world_with(
            &blocks,
            core::slice::from_ref(&gentle),
            core::slice::from_ref(&platform),
        );
        let mut w = Walker::spawn(21, [0, 0, 0]);
        for tick in 0..300 {
            let input = TickInput {
                wish_mm_s: [3_000, 0],
                jump_pressed: tick == 60,
            };
            step_walker(&mut w, &input, &world, &cfg);
        }
        let restored = w;
        for tick in 300..600 {
            let input = TickInput {
                wish_mm_s: [3_000, 0],
                jump_pressed: tick == 60,
            };
            step_walker(&mut w, &input, &world, &cfg);
        }
        assert_eq!(w, restored_step(&world, &cfg, restored));
        assert_eq!(state_hash(&w), state_hash(&first));
    }

    fn restored_step(world: &StepWorld<'_>, cfg: &Config, mut w: Walker) -> Walker {
        for tick in 300..600 {
            let input = TickInput {
                wish_mm_s: [3_000, 0],
                jump_pressed: tick == 60,
            };
            step_walker(&mut w, &input, world, cfg);
        }
        w
    }

    /// Walk `ticks` on the flat slab at a steady wish, returning the walker.
    fn walk_at(wish_mm_s: [i32; 2], ticks: u32) -> Walker {
        let cfg = Config::default();
        let blocks = slab();
        let world = world_with(&blocks, &[], &[]);
        let mut w = Walker::spawn(41, [0, 0, 0]);
        let input = TickInput {
            wish_mm_s,
            jump_pressed: false,
        };
        for _ in 0..ticks {
            step_walker(&mut w, &input, &world, &cfg);
        }
        w
    }

    #[test]
    fn slow_walks_cover_their_true_distance() {
        // Law: distance = speed x time at any speed. Truncating vel/60 stalled 59 mm/s and
        // walked 100 mm/s at 60. Past the ramp-up (one second) the walker holds the wish, so
        // the next ten seconds cover 10 x speed millimetres, give or take the carried remainder.
        for speed in [30, 59, 100, 1_000, 2_999] {
            let start = walk_at([speed, 0], 60).pos_mm[0];
            let end = walk_at([speed, 0], 660).pos_mm[0];
            let want = speed * 10;
            assert!(
                (end - start - want).abs() <= 1,
                "{speed} mm/s for 10 s moved {} mm, want {want}",
                end - start
            );
        }
    }

    #[test]
    fn a_walker_at_rest_stays_exactly_at_rest() {
        let w = walk_at([0, 0], 600);
        assert_eq!(w.pos_mm, [0, 0, 0]);
        assert_eq!(w.vel_mm_s, [0, 0, 0]);
        // Stopping keeps a sub-millimetre carry; it alone never moves the walker.
        let cfg = Config::default();
        let blocks = slab();
        let world = world_with(&blocks, &[], &[]);
        let mut w = walk_at([1_030, 0], 61);
        for _ in 0..30 {
            step_walker(&mut w, &TickInput::idle(), &world, &cfg);
        }
        let parked = w.pos_mm;
        for _ in 0..600 {
            step_walker(&mut w, &TickInput::idle(), &world, &cfg);
        }
        assert_eq!(w.pos_mm, parked, "a stopped walker crept: {w:?}");
    }

    #[test]
    fn mirrored_input_gives_a_mirrored_walk() {
        for wish in [[1_037, 0], [0, 777], [2_345, -1_234], [41, 59]] {
            let a = walk_at(wish, 300);
            let b = walk_at([-wish[0], -wish[1]], 300);
            assert_eq!(a.pos_mm[0], -b.pos_mm[0], "x not mirrored for {wish:?}");
            assert_eq!(a.pos_mm[2], -b.pos_mm[2], "z not mirrored for {wish:?}");
            assert_eq!(a.pos_mm[1], b.pos_mm[1]);
        }
    }

    #[test]
    fn steep_slopes_are_solid_and_slide_downhill_both_ways() {
        let cfg = Config::default();
        let blocks = slab();
        // The same 56-degree face rising toward +x, authored three ways: +x rise, a high-x-first
        // run, and the mirror descending toward +x. Each must hold the walker on its face and
        // slide it toward the low end, never drop it through.
        let rising = Ramp {
            x0_mm: 2_000,
            x1_mm: 6_000,
            z0_mm: -2_000,
            z1_mm: 2_000,
            y_base_mm: 0,
            rise_mm: 6_000,
        };
        let reversed = Ramp {
            x0_mm: 6_000,
            x1_mm: 2_000,
            z0_mm: -2_000,
            z1_mm: 2_000,
            y_base_mm: 6_000,
            rise_mm: -6_000,
        };
        let falling = Ramp {
            x0_mm: -6_000,
            x1_mm: -2_000,
            z0_mm: -2_000,
            z1_mm: 2_000,
            y_base_mm: 6_000,
            rise_mm: -6_000,
        };
        for (ramp, start_x, downhill) in [
            (rising, 4_000, -1),
            (reversed, 4_000, -1),
            (falling, -4_000, 1),
        ] {
            let ramps = [ramp];
            let world = world_with(&blocks, &ramps, &[]);
            let face = ramp.surface_at(start_x);
            assert_eq!(face, 3_000, "{ramp:?} face height");
            let mut w = Walker::spawn(9, [start_x, face + 100, 0]);
            let mut slipped = false;
            for _ in 0..20 {
                let r = step_walker(&mut w, &TickInput::idle(), &world, &cfg);
                slipped |= r.slipped;
                let under = ramp.surface_at(w.pos_mm[0]);
                if ramp.contains(w.pos_mm[0], w.pos_mm[2]) {
                    assert!(
                        w.pos_mm[1] >= under - cfg.snap_mm,
                        "fell through {ramp:?}: {w:?}"
                    );
                }
            }
            assert!(slipped, "{ramp:?} never slipped: {w:?}");
            assert_eq!(
                (w.pos_mm[0] - start_x).signum(),
                downhill,
                "{ramp:?} slid uphill: {w:?}"
            );
        }
    }

    #[test]
    fn step_allocates_nothing() {
        let cfg = Config::default();
        let blocks = slab();
        let world = world_with(&blocks, &[], &[]);
        let mut w = Walker::spawn(31, [0, 0, 500]);
        let input = TickInput {
            wish_mm_s: [1_000, 0],
            jump_pressed: false,
        };
        let ((), tick_allocs) = alloc_probe::measure(|| {
            step_walker(&mut w, &input, &world, &cfg);
        });
        assert_eq!(tick_allocs.calls, 0);
        let sense = sense_ahead(&w, 500, 0, &world, &cfg);
        let _ = sense;
        let (_, hash_allocs) = alloc_probe::measure(|| state_hash(&w));
        assert_eq!(hash_allocs.calls, 0);
    }
}
