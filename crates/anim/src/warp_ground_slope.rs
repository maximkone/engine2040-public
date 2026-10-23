//! warp_ground_slope: slope cling, stair stepping and soft-ground sink beside
//! the GASP ground warp.
//!
//! Owner: `engine-anim`; consumers: arena presentation wraps each slope tick
//! through [`SlopeWarp`] and feeds stair or sand contacts through
//! [`GroundFeet`]. [`warp_ground`](super::warp_ground) stays the one distance,
//! orientation and stride warp; this module never runs a second one.
//!
//! * slope cling: [`SlopeWarp`] feeds [`GroundWarp::warp_output`] the surface
//!   speed `capsule * sec(grade)`, so planted feet cover the incline instead of
//!   its horizontal projection; the 20 degree grade is [`SLOPE_20_GRADE_Q`];
//! * stair stepping: [`GroundFeet`] snaps each plant to the tread under it,
//!   re-anchors when a planted foot crosses a riser (a counted step, not
//!   slide) and reports the lowest planted tread for the pelvis drop;
//! * soft ground: [`Sand`] sinks the contact by an integer settle curve, and
//!   [`GroundFeet`] keeps the plant x/z fixed, so sink reads as sink, never as
//!   skate.
//!
//! Units: meters and seconds at 60 Hz on the float lane; grades in per-mille,
//! runs, treads, sinks and contacts in integer millimeters. Determinism:
//! integer stair/sand state plus `+ - * /` on the slope feed; no clock, no
//! heap, stable foot order. Errors are [`SlopeError`], never panics; a
//! rejected tick mutates nothing. Budget: O(1) per foot per tick plus the
//! wrapped warp's O(joints) pose writes.

use super::locomotion::{LocoOutput, LocoState, MAX_WISH_SPEED};
use super::warp_ground::{GroundWarp, WarpError, WarpReceipt};
use engine_core::checked::f32_from_u64;
use engine_core::checked::saturating_i64_to_i32;
use engine_core::fixed::isqrt;
use engine_core::Fnv64;

/// 20 degree grade in per-mille rise over run: `tan(20 deg) * 1000`, rounded.
pub const SLOPE_20_GRADE_Q: i32 = 364;
/// Steepest supported grade in per-mille: 45 degrees, the physics cap.
pub const MAX_GRADE_Q: i32 = 1000;
/// Steepest supported stair riser in millimeters, matching the physics step.
pub const MAX_STAIR_RISE_MM: i32 = 500;
/// Height (mm) of the contact point above the surface it stands on.
pub const SOLE_MM: i32 = 30;
/// Deepest soft-ground sink the planter accepts, in millimeters.
pub const MAX_SINK_MM: i32 = 200;

/// Slope, stair and sand failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SlopeError {
    /// The wrapped ground warp rejected the tick.
    Warp(WarpError),
    /// Grade exceeded [`MAX_GRADE_Q`] in either direction; value carried.
    GradeTooSteep(i32),
    /// Surface speed exceeded [`MAX_WISH_SPEED`] after the grade factor;
    /// value carried.
    SurfaceTooFast(f32),
    /// A stair lane was illegal; the lane names the problem.
    BadStair(&'static str),
    /// A sand lane was illegal; the lane names the problem.
    BadSand(&'static str),
    /// Sink was outside `0..=`[`MAX_SINK_MM`]; value carried.
    BadSink(i32),
}

impl core::fmt::Display for SlopeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SlopeError::Warp(err) => write!(f, "slope: {err}"),
            SlopeError::GradeTooSteep(q) => {
                write!(f, "slope: grade {q} past the {MAX_GRADE_Q} per-mille cap")
            }
            SlopeError::SurfaceTooFast(v) => {
                write!(f, "slope: surface speed {v} past the wish limit")
            }
            SlopeError::BadStair(lane) => write!(f, "stair: bad {lane}"),
            SlopeError::BadSand(lane) => write!(f, "sand: bad {lane}"),
            SlopeError::BadSink(v) => write!(f, "sand: sink {v} mm out of range"),
        }
    }
}

impl std::error::Error for SlopeError {}

impl From<WarpError> for SlopeError {
    fn from(err: WarpError) -> Self {
        Self::Warp(err)
    }
}

/// Slope grade as integer per-mille rise over run; negative is downhill.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Grade {
    grade_q: i32,
    sec_q: u32,
}

impl Grade {
    /// Flat ground: grade 0, secant 1000.
    pub const FLAT: Grade = Grade {
        grade_q: 0,
        sec_q: 1000,
    };

    /// Grade from per-mille rise over run; refuses past [`MAX_GRADE_Q`].
    pub fn new(grade_q: i32) -> Result<Self, SlopeError> {
        if !(-MAX_GRADE_Q..=MAX_GRADE_Q).contains(&grade_q) {
            return Err(SlopeError::GradeTooSteep(grade_q));
        }
        let q = i128::from(grade_q);
        let d2 = 1_000_000 + q * q;
        let sec = isqrt(u128::try_from(d2).unwrap_or(u128::MAX));
        Ok(Self {
            grade_q,
            sec_q: u32::try_from(sec).unwrap_or(u32::MAX),
        })
    }

    /// Per-mille rise over run; negative is downhill.
    pub fn grade_q(self) -> i32 {
        self.grade_q
    }

    /// `sqrt(1 + grade^2) * 1000`: 1000 on flat, 1064 at 20 degrees.
    pub fn sec_q(self) -> u32 {
        self.sec_q
    }

    /// True on flat ground.
    pub fn is_flat(self) -> bool {
        self.grade_q == 0
    }

    /// Unit up-slope direction `[x, y, z]` for travel toward `+Z`; `x` is 0.
    pub fn surface_dir(self) -> [f32; 3] {
        let sec = f32_from_u64(u64::from(self.sec_q));
        let q = f32_from_u64(u64::from(self.grade_q.unsigned_abs()));
        let sign = if self.grade_q < 0 { -1.0 } else { 1.0 };
        [0.0, sign * q / sec, 1000.0 / sec]
    }

    /// Surface speed (m/s) for a planar speed: `planar * sec`.
    pub fn surface_speed_m_s(self, planar_m_s: f32) -> f32 {
        planar_m_s * f32_from_u64(u64::from(self.sec_q)) / 1000.0
    }
}

impl Default for Grade {
    fn default() -> Self {
        Self::FLAT
    }
}

/// One accepted slope tick: the wrapped receipt plus the grade factors used.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SlopeReceipt {
    /// The wrapped ground warp receipt (rate, stride, yaw, clip advance).
    pub warp: WarpReceipt,
    /// Per-mille grade applied this tick; negative is downhill.
    pub grade_q: i32,
    /// Grade secant in per-mille (1000 flat, 1064 at 20 degrees).
    pub sec_q: u32,
    /// Surface speed (m/s) fed to the wrapped warp this tick.
    pub surface_speed_m_s: f32,
}

/// One fighter's slope ground feel: the one [`GroundWarp`] plus the grade
/// factor its capsule speed rides. Construct with [`SlopeWarp::new`], open
/// transients with [`SlopeWarp::begin_transient`], then post-process every
/// locomotion output with [`SlopeWarp::warp_output`]. The root passes through
/// bit-identical; only the presentation advances faster along the surface.
/// `Clone` snapshots the full warp state for rollback or repeat checks.
#[derive(Clone, PartialEq, Debug)]
pub struct SlopeWarp {
    warp: GroundWarp,
    grade: Grade,
    surface_speed_m_s: f32,
}

impl SlopeWarp {
    /// New warp on `grade`: no open transient, surface speed 0.
    pub fn new(grade: Grade) -> Self {
        Self {
            warp: GroundWarp::new(),
            grade,
            surface_speed_m_s: 0.0,
        }
    }

    /// The grade this warp clings to.
    pub fn grade(&self) -> Grade {
        self.grade
    }

    /// The wrapped ground warp, read-only (state and receipts).
    pub fn warp(&self) -> &GroundWarp {
        &self.warp
    }

    /// Surface speed (m/s) applied on the latest accepted tick.
    pub fn surface_speed_m_s(&self) -> f32 {
        self.surface_speed_m_s
    }

    /// Open a transient window like [`GroundWarp::begin_transient`], but treat
    /// `capsule_speed` as planar and scale it by the grade secant here, so the
    /// rate matches the surface the planted feet must cover. Rejects bad
    /// speeds and over-limit surface speeds without touching state.
    pub fn begin_transient(
        &mut self,
        state: LocoState,
        dur_ticks: u32,
        target_speed: f32,
        capsule_speed: f32,
    ) -> Result<(), SlopeError> {
        let surface = self.surface_for(capsule_speed)?;
        self.warp
            .begin_transient(state, dur_ticks, target_speed, surface)?;
        self.surface_speed_m_s = surface;
        Ok(())
    }

    /// Post-process one locomotion output on the slope: feeds the wrapped warp
    /// the surface speed, then delegates the rate, stride, orientation and clip
    /// advance. The root passes through bit-identical. Atomic: any rejection
    /// leaves both the output and the warp untouched. Writes in place only,
    /// never allocates.
    pub fn warp_output(
        &mut self,
        out: &mut LocoOutput,
        capsule_speed: f32,
        wish_dir: [f32; 2],
        legs: &[usize],
    ) -> Result<SlopeReceipt, SlopeError> {
        let surface = self.surface_for(capsule_speed)?;
        let warp = self.warp.warp_output(out, surface, wish_dir, legs)?;
        self.surface_speed_m_s = surface;
        Ok(SlopeReceipt {
            warp,
            grade_q: self.grade.grade_q(),
            sec_q: self.grade.sec_q(),
            surface_speed_m_s: surface,
        })
    }

    /// Planar capsule speed to surface speed, with the wish-limit law shared by
    /// the wrapped warp. Checks the planar lane first so a bad caller value
    /// reports the same typed error on flat and sloped ground.
    fn surface_for(&self, capsule_speed: f32) -> Result<f32, SlopeError> {
        if !capsule_speed.is_finite() {
            return Err(SlopeError::Warp(WarpError::NonFinite("capsule")));
        }
        if capsule_speed < 0.0 {
            return Err(SlopeError::Warp(WarpError::NegativeSpeed(capsule_speed)));
        }
        if capsule_speed > MAX_WISH_SPEED {
            return Err(SlopeError::Warp(WarpError::SpeedTooHigh(capsule_speed)));
        }
        let surface = self.grade.surface_speed_m_s(capsule_speed);
        if surface > MAX_WISH_SPEED {
            return Err(SlopeError::SurfaceTooFast(surface));
        }
        Ok(surface)
    }
}

/// One straight stair flight: a constant riser and tread in millimeters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StairFlight {
    rise_mm: i32,
    tread_mm: i32,
}

impl StairFlight {
    /// Flight with `rise_mm` per `tread_mm`: rise in `1..=`[`MAX_STAIR_RISE_MM`]
    /// and tread at least the rise (45 degrees or shallower).
    pub fn new(rise_mm: i32, tread_mm: i32) -> Result<Self, SlopeError> {
        if !(1..=MAX_STAIR_RISE_MM).contains(&rise_mm) {
            return Err(SlopeError::BadStair("rise"));
        }
        if tread_mm < rise_mm {
            return Err(SlopeError::BadStair("tread"));
        }
        Ok(Self { rise_mm, tread_mm })
    }

    /// Riser height in millimeters.
    pub fn rise_mm(self) -> i32 {
        self.rise_mm
    }

    /// Tread depth in millimeters.
    pub fn tread_mm(self) -> i32 {
        self.tread_mm
    }

    /// Tread index under a run: `floor(run / tread)`, so run 0 is the first
    /// tread and negative runs step below the origin.
    pub fn tread_index(self, run_mm: i64) -> i64 {
        run_mm.div_euclid(i64::from(self.tread_mm))
    }

    /// Tread top under a run, in millimeters from the first tread.
    pub fn height_mm(self, run_mm: i64) -> i64 {
        self.tread_index(run_mm) * i64::from(self.rise_mm)
    }
}

/// Soft ground: a fixed sink depth (mm) reached after `settle_ticks` planted
/// ticks. The curve is integer linear, so replay is exact.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sand {
    depth_mm: i32,
    settle_ticks: u32,
}

impl Sand {
    /// Sand with `depth_mm` in `1..=`[`MAX_SINK_MM`] and a non-zero settle
    /// window.
    pub fn new(depth_mm: i32, settle_ticks: u32) -> Result<Self, SlopeError> {
        if !(1..=MAX_SINK_MM).contains(&depth_mm) {
            return Err(SlopeError::BadSand("depth"));
        }
        if settle_ticks == 0 {
            return Err(SlopeError::BadSand("settle"));
        }
        Ok(Self {
            depth_mm,
            settle_ticks,
        })
    }

    /// Fully settled sink depth in millimeters.
    pub fn depth_mm(self) -> i32 {
        self.depth_mm
    }

    /// Ticks the contact takes to reach the full depth.
    pub fn settle_ticks(self) -> u32 {
        self.settle_ticks
    }

    /// Sink (mm) after `plant_ticks` planted ticks: linear integer settle,
    /// holding `depth_mm` once settled.
    pub fn sink_mm(self, plant_ticks: u32) -> i32 {
        let age = i64::from(plant_ticks.min(self.settle_ticks));
        let depth = i64::from(self.depth_mm);
        let settle = i64::from(self.settle_ticks);
        saturating_i64_to_i32(depth * age / settle)
    }
}

/// Euclidean distance in integer millimeters: one integer square root, no
/// floats, so a meter can never be steered by a rounding mode.
pub fn dist_mm(a: [i32; 3], b: [i32; 3]) -> u32 {
    let dx = i128::from(a[0]) - i128::from(b[0]);
    let dy = i128::from(a[1]) - i128::from(b[1]);
    let dz = i128::from(a[2]) - i128::from(b[2]);
    let d2 = dx * dx + dy * dy + dz * dz;
    let d = isqrt(u128::try_from(d2).unwrap_or(0));
    u32::try_from(d).unwrap_or(u32::MAX)
}

/// One stair/sand ground tick for both feet, integer millimeters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GroundTick {
    /// Contact point per foot `[x, y, z]` in millimeters (surface + sole - sink).
    pub contact_mm: [[i32; 3]; 2],
    /// Planar slide from the plant anchor per foot, in millimeters.
    pub slide_mm: [u32; 2],
    /// Soft-ground sink applied to the contact per foot, in millimeters.
    pub sink_mm: [i32; 2],
    /// Tread index the foot stands on per foot.
    pub tread: [i64; 2],
    /// True while the foot holds a plant anchor.
    pub locked: [bool; 2],
}

/// Plant anchor of one foot: world x/z plus the tread it stands on. The y lane
/// is re-derived each tick from the tread and the sink, so sink never reads as
/// skate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Anchor {
    x_mm: i32,
    z_mm: i32,
    tread: i64,
}

/// Stair- and sand-aware planter and slide meter for both feet.
///
/// Each planted foot holds a world x/z anchor on the tread under its raw run.
/// A raw run that crosses a riser re-anchors to the new tread and counts a step
/// event instead of dragging the foot through the riser. Sink lowers both the
/// raw contact and the held anchor, so it never reads as skate. Feet run in
/// stable order (`0` then `1`); the state is integer-only and replays exactly.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GroundFeet {
    flight: StairFlight,
    anchors: [Option<Anchor>; 2],
    last_tread: [Option<i64>; 2],
    plants: u32,
    steps_up: u32,
    steps_down: u32,
}

impl GroundFeet {
    /// New planter over `flight`; no anchors and no plants.
    pub fn new(flight: StairFlight) -> Self {
        Self {
            flight,
            anchors: [None, None],
            last_tread: [None, None],
            plants: 0,
            steps_up: 0,
            steps_down: 0,
        }
    }

    /// The flight this planter reads.
    pub fn flight(&self) -> StairFlight {
        self.flight
    }

    /// New plants anchored so far: the first tick of each down run.
    pub fn plants(&self) -> u32 {
        self.plants
    }

    /// Step events upward: a new plant or a mid-stance re-anchor on a higher
    /// tread counts once per event, not once per riser.
    pub fn steps_up(&self) -> u32 {
        self.steps_up
    }

    /// Step events downward, counted like [`GroundFeet::steps_up`].
    pub fn steps_down(&self) -> u32 {
        self.steps_down
    }

    /// Lowest tread top under a planted foot, in millimeters; `None` when
    /// neither foot is planted. The pelvis may drop to this height so the body
    /// follows the lower foot across a riser.
    pub fn pelvis_ground_mm(&self) -> Option<i64> {
        self.anchors
            .iter()
            .flatten()
            .map(|a| self.flight.height_mm(i64::from(a.z_mm)))
            .min()
    }

    /// One tick for both feet. `raw_mm` is each foot's raw clip contact; its
    /// x/z decide the plant and its y lane is re-derived from the tread and
    /// sink. `planted` is the gait contact; `sink_mm` is the soft-ground depth
    /// per foot. A planted foot snaps to the tread under its raw run and holds
    /// its anchor; a raw run that crosses a riser steps the anchor to the new
    /// tread. Slide meters the remaining planar drift. Rejects a sink outside
    /// `0..=`[`MAX_SINK_MM`] without touching state.
    pub fn tick(
        &mut self,
        raw_mm: [[i32; 3]; 2],
        planted: [bool; 2],
        sink_mm: [i32; 2],
    ) -> Result<GroundTick, SlopeError> {
        for sink in sink_mm {
            if !(0..=MAX_SINK_MM).contains(&sink) {
                return Err(SlopeError::BadSink(sink));
            }
        }
        let mut tick = GroundTick {
            contact_mm: [[0; 3]; 2],
            slide_mm: [0; 2],
            sink_mm,
            tread: [0; 2],
            locked: [false; 2],
        };
        for foot in 0..2 {
            let (contact, slide, tread, locked) =
                self.step_foot(foot, raw_mm[foot], planted[foot], sink_mm[foot]);
            tick.contact_mm[foot] = contact;
            tick.slide_mm[foot] = slide;
            tick.tread[foot] = tread;
            tick.locked[foot] = locked;
        }
        Ok(tick)
    }

    /// One foot's tick: hold, re-anchor across a riser, or release.
    fn step_foot(
        &mut self,
        foot: usize,
        raw: [i32; 3],
        planted: bool,
        sink: i32,
    ) -> ([i32; 3], u32, i64, bool) {
        let raw_tread = self.flight.tread_index(i64::from(raw[2]));
        if !planted {
            self.anchors[foot] = None;
            let y = self.surface_y(raw_tread, sink);
            return ([raw[0], y, raw[2]], 0, raw_tread, false);
        }
        match self.anchors[foot] {
            None => {
                self.anchor_new(foot, raw, raw_tread);
                let y = self.surface_y(raw_tread, sink);
                ([raw[0], y, raw[2]], 0, raw_tread, true)
            }
            Some(anchor) if anchor.tread == raw_tread => {
                let y = self.surface_y(anchor.tread, sink);
                let contact = [anchor.x_mm, y, anchor.z_mm];
                let raw_contact = [raw[0], y, raw[2]];
                (contact, dist_mm(contact, raw_contact), anchor.tread, true)
            }
            Some(anchor) => {
                self.count_step(foot, anchor.tread, raw_tread);
                self.anchors[foot] = Some(Anchor {
                    x_mm: raw[0],
                    z_mm: raw[2],
                    tread: raw_tread,
                });
                let y = self.surface_y(raw_tread, sink);
                ([raw[0], y, raw[2]], 0, raw_tread, true)
            }
        }
    }

    /// Anchor a fresh plant and count the step from the foot's last plant.
    fn anchor_new(&mut self, foot: usize, raw: [i32; 3], tread: i64) {
        self.anchors[foot] = Some(Anchor {
            x_mm: raw[0],
            z_mm: raw[2],
            tread,
        });
        if let Some(last) = self.last_tread[foot] {
            self.count_step(foot, last, tread);
        }
        self.last_tread[foot] = Some(tread);
        self.plants += 1;
    }

    /// Count one step event and remember the tread this foot stands on.
    fn count_step(&mut self, foot: usize, from: i64, to: i64) {
        if to > from {
            self.steps_up += 1;
        } else if to < from {
            self.steps_down += 1;
        }
        self.last_tread[foot] = Some(to);
    }

    /// Contact height for a tread: tread top plus the sole, less the sink.
    fn surface_y(&self, tread: i64, sink: i32) -> i32 {
        let top = tread
            .saturating_mul(i64::from(self.flight.rise_mm()))
            .saturating_add(i64::from(SOLE_MM))
            .saturating_sub(i64::from(sink));
        saturating_i64_to_i32(top)
    }
}

/// FNV-1a64 over one ground tick (contacts, slides, sinks, treads, locks); the
/// replay receipt for stair and sand fixtures.
pub fn ground_hash(tick: &GroundTick) -> u64 {
    let mut h = Fnv64::new();
    for foot in 0..2 {
        for lane in tick.contact_mm[foot] {
            h.write_i64(i64::from(lane));
        }
        h.write_u32(tick.slide_mm[foot]);
        h.write_i64(i64::from(tick.sink_mm[foot]));
        h.write_i64(tick.tread[foot]);
        h.write_u32(u32::from(u8::from(tick.locked[foot])));
    }
    h.finish()
}

/// FNV-1a64 over a ground tick sequence: equal scripts hash equally, the
/// replay receipt for stair and sand runs.
pub fn ground_replay_hash(ticks: &[GroundTick]) -> u64 {
    let mut h = Fnv64::new();
    h.write_len(ticks.len());
    for tick in ticks.iter() {
        h.write_u64(ground_hash(tick));
    }
    h.finish()
}
