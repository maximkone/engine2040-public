//! readable: facing arrow plus lean for locomotion presentation.
//!
//! Owner: `engine-anim`; status: presentation overlay. The production
//! caller is [`crate::locomotion::Locomotion::drive_readable`]: it drives
//! [`Readable::update`] from the machine facing and hands the frame to HUD
//! and capture. UNLOCKS: the crowd-readability follow-up (readable far-ring
//! at LOD, beside `locomotion.rs`, lands after this) reads
//! [`ReadableFrame::arrow`] (unit `[x, z]` facing
//! arrow) and [`ReadableFrame::lean`] (forward lean in radians) as a
//! presentation overlay beside [`crate::locomotion::LocoOutput::pose`],
//! with HUD truth wiring after that. Simulation never reads this module:
//! root intent and contacts stay authoritative in
//! `locomotion.rs`, so the golden tick hash cannot move. Units: meters,
//! seconds, radians; fixed 60 Hz ticks. Determinism: `+ - * /` plus `sqrt`
//! only, no transcendentals, no wall clock, stable order; the same input
//! script twice gives identical frames. Errors: [`ReadableError`].

/// Max arrow turn per tick (radians of yaw, chord-metered).
pub const ARROW_MAX_TURN: f32 = 0.12;
/// Forward lean per planar speed (rad per m/s).
pub const LEAN_PER_SPEED: f32 = 0.022;
/// Largest forward lean (radians).
pub const LEAN_MAX: f32 = 0.15;
/// Max lean change per tick (radians): velocity continuity.
pub const LEAN_SLEW: f32 = 0.02;
/// Below this planar speed (m/s) the arrow holds facing.
pub const IDLE_SPEED: f32 = 0.05;
/// Above this planar speed (m/s) input is refused, never clamped.
pub const MAX_SPEED: f32 = 12.0;
/// Closest the two feet may read (meters): no snap-together.
pub const MIN_FOOT_SEP_M: f32 = 0.08;
/// Planted-foot slide budget (integer millimeters): under 1 cm.
pub const SLIDE_BUDGET_MM: u32 = 10;

/// Bounded readable-input failures: typed, never panics.
#[derive(Clone, PartialEq, Debug)]
pub enum ReadableError {
    /// A lane of the named input was not finite.
    NonFiniteInput(&'static str),
    /// Facing was zero-length while the arrow needed it.
    BadFacing,
    /// Planar speed exceeded [`MAX_SPEED`]; value carried.
    SpeedTooHigh(f32),
}

impl core::fmt::Display for ReadableError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReadableError::NonFiniteInput(lane) => {
                write!(f, "readable: non-finite float in {lane}")
            }
            ReadableError::BadFacing => write!(f, "readable: facing has no direction"),
            ReadableError::SpeedTooHigh(v) => {
                write!(f, "readable: speed {v} above {MAX_SPEED}")
            }
        }
    }
}

impl std::error::Error for ReadableError {}

/// One presentation frame: arrow plus lean for this tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ReadableFrame {
    /// Unit `[x, z]` arrow: velocity direction when moving, facing at rest.
    pub arrow: [f32; 2],
    /// Forward lean in radians in `[0, LEAN_MAX]`.
    pub lean: f32,
    /// Planar speed (m/s) accepted this tick.
    pub speed: f32,
}

/// Slew-limited arrow plus lean. Construct with [`Readable::new`], drive
/// one fixed tick per [`Readable::update`]. `Clone` snapshots full state.
#[derive(Clone, PartialEq, Debug)]
pub struct Readable {
    arrow: [f32; 2],
    lean: f32,
}

fn normalize2(v: [f32; 2]) -> Option<[f32; 2]> {
    let len2 = v[0] * v[0] + v[1] * v[1];
    if !len2.is_finite() || len2 <= 0.0 {
        return None;
    }
    let inv = 1.0 / len2.sqrt();
    let out = [v[0] * inv, v[1] * inv];
    if !out[0].is_finite() || !out[1].is_finite() {
        return None;
    }
    Some(out)
}

/// Chord between unit arrows: `|a - b|`, which bounds yaw (`chord <= yaw`
/// for small angles) with `+ - *` plus `sqrt` only.
pub fn arrow_chord(a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = a[0] - b[0];
    let dz = a[1] - b[1];
    (dx * dx + dz * dz).sqrt()
}

/// Planted-foot slide meter, in integer millimeters. Reused from the foot-lock
/// owner so there is exactly one slide path in the crate.
pub use super::feet::slide_mm;

/// True when the feet read apart: separation at least [`MIN_FOOT_SEP_M`].
pub fn feet_apart(left: [f32; 3], right: [f32; 3]) -> bool {
    let dx = left[0] - right[0];
    let dy = left[1] - right[1];
    let dz = left[2] - right[2];
    let d2 = dx * dx + dy * dy + dz * dz;
    if !d2.is_finite() {
        return false;
    }
    d2 >= MIN_FOOT_SEP_M * MIN_FOOT_SEP_M
}

impl Readable {
    /// New overlay: arrow starts on `facing`, lean at rest. A degenerate
    /// facing binds to `+Z` so construction is total.
    pub fn new(facing: [f32; 2]) -> Self {
        Self {
            arrow: normalize2(facing).unwrap_or([0.0, 1.0]),
            lean: 0.0,
        }
    }

    /// Current arrow (unit `[x, z]`).
    pub fn arrow(&self) -> [f32; 2] {
        self.arrow
    }

    /// Current lean (radians).
    pub fn lean(&self) -> f32 {
        self.lean
    }

    /// One presentation tick. `mx`/`mz` carry planar velocity (m/s) in
    /// world `[x, z]`; `facing` carries the body facing (unit `[x, z]`).
    /// Moving ticks steer the arrow toward velocity, resting ticks toward
    /// facing, both capped at [`ARROW_MAX_TURN`]; lean tracks speed at
    /// [`LEAN_SLEW`] per tick. Rejected input leaves state untouched.
    /// Allocation-free: only `Copy` scalars move here.
    pub fn update(
        &mut self,
        mx: f32,
        mz: f32,
        facing: [f32; 2],
    ) -> Result<ReadableFrame, ReadableError> {
        if !mx.is_finite() {
            return Err(ReadableError::NonFiniteInput("mx"));
        }
        if !mz.is_finite() {
            return Err(ReadableError::NonFiniteInput("mz"));
        }
        if !facing.iter().all(|v| v.is_finite()) {
            return Err(ReadableError::NonFiniteInput("facing"));
        }
        let speed2 = mx * mx + mz * mz;
        if !speed2.is_finite() {
            return Err(ReadableError::NonFiniteInput("mx/mz"));
        }
        let speed = speed2.sqrt();
        if !speed.is_finite() {
            return Err(ReadableError::NonFiniteInput("speed"));
        }
        if speed > MAX_SPEED {
            return Err(ReadableError::SpeedTooHigh(speed));
        }
        let target = if speed < IDLE_SPEED {
            normalize2(facing).ok_or(ReadableError::BadFacing)?
        } else {
            // Finite nonzero speed here, so this cannot fail; bind anyway.
            normalize2([mx, mz]).ok_or(ReadableError::BadFacing)?
        };
        // Slew the arrow toward the target, capped per tick so a reversal
        // or strafe flick never snaps the overlay. Small-angle yaw step:
        // `sin(d) ~ d`, `cos(d) ~ 1 - d^2/2`, valid because
        // `|dyaw| <= ARROW_MAX_TURN` (0.12): `|sin(d) - d| <= |d|^3/6`
        // stays under 2.9e-4 and `|cos(d) - (1 - d^2/2)| <= d^4/24`
        // stays under 8.7e-6 against real `sin`/`cos` (pinned by
        // `yaw_step_matches_sin_cos`). Taylor form keeps the tick to
        // `+ - * /` plus `sqrt`, with no transcendentals in production.
        let cross = self.arrow[0] * target[1] - self.arrow[1] * target[0];
        let dyaw = (-cross).clamp(-ARROW_MAX_TURN, ARROW_MAX_TURN);
        let sin = dyaw;
        let cos = 1.0 - dyaw * dyaw * 0.5;
        let a = self.arrow;
        let stepped =
            normalize2([a[0] * cos + a[1] * sin, -a[0] * sin + a[1] * cos]).unwrap_or(self.arrow);
        self.arrow = stepped;
        // Slew the lean toward the speed-scaled target for continuity.
        let want = (speed * LEAN_PER_SPEED).clamp(0.0, LEAN_MAX);
        let d = (want - self.lean).clamp(-LEAN_SLEW, LEAN_SLEW);
        self.lean += d;
        Ok(ReadableFrame {
            arrow: self.arrow,
            lean: self.lean,
            speed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    /// Seed95 ticks20-30 shape: steady run, then a diagonal strafe flick,
    /// then rest. Eleven ticks like the playtest56 window.
    fn seed95_window() -> Vec<(f32, f32, [f32; 2])> {
        let face = [0.0, 1.0];
        vec![
            (0.0, 3.8, face),
            (0.0, 3.8, face),
            (0.0, 3.8, face),
            (1.4, 3.4, face),
            (2.6, 2.4, face),
            (3.2, 0.6, face),
            (3.4, -0.4, face),
            (1.2, -0.6, face),
            (0.0, 0.0, face),
            (0.0, 0.0, face),
            (0.0, 0.0, face),
        ]
    }

    /// Drive the window, checking per-tick continuity bars. Returns frames.
    fn drive_window() -> Vec<ReadableFrame> {
        let script = seed95_window();
        let mut r = Readable::new([0.0, 1.0]);
        let mut prev_arrow = r.arrow();
        let mut prev_lean = r.lean();
        let left0 = [0.11, 0.0, 0.20];
        let right0 = [-0.11, 0.0, -0.18];
        assert!(feet_apart(left0, right0), "fixture feet start apart");
        let mut frames = Vec::with_capacity(script.len());
        for (mx, mz, face) in &script {
            let fr = r.update(*mx, *mz, *face).unwrap();
            // Arrow continuity: chord per tick within the slew cap plus
            // the small-angle step epsilon (measured 2e-4, far from a snap).
            let chord = arrow_chord(prev_arrow, fr.arrow);
            assert!(
                chord <= ARROW_MAX_TURN + 1e-3,
                "arrow snaps: chord={chord} {prev_arrow:?} -> {:?}",
                fr.arrow
            );
            // Lean continuity and range.
            assert!(
                (fr.lean - prev_lean).abs() <= LEAN_SLEW + 1e-6,
                "lean jumps: {} -> {}",
                prev_lean,
                fr.lean
            );
            assert!(
                (0.0..=LEAN_MAX + 1e-6).contains(&fr.lean),
                "lean out of range: {}",
                fr.lean
            );
            // Arrow stays unit length (readable at any zoom).
            let len = (fr.arrow[0] * fr.arrow[0] + fr.arrow[1] * fr.arrow[1]).sqrt();
            assert!((len - 1.0).abs() < 1e-5, "arrow not unit: {:?}", fr.arrow);
            // Planted feet hold: slide of a held anchor reads zero, and
            // feet never snap together on this window.
            assert_eq!(slide_mm(left0, left0), 0);
            assert!(feet_apart(left0, right0), "feet snapped together");
            prev_arrow = fr.arrow;
            prev_lean = fr.lean;
            frames.push(fr);
        }
        frames
    }

    #[test]
    fn seed95_ticks20_30_arrow_lean_slide_no_snap() {
        let frames = drive_window();
        // A 5 mm planted drift stays inside the 1 cm budget.
        let drifted = slide_mm([0.11, 0.0, 0.20], [0.115, 0.0, 0.20]);
        assert_eq!(drifted, 5);
        assert!(drifted <= SLIDE_BUDGET_MM, "slide={drifted}");
        // The window actually moves the overlay: arrow turns, lean rises,
        // then lean settles back at rest.
        assert!(
            arrow_chord(frames[0].arrow, frames[6].arrow) > 0.05,
            "arrow never turns: {:?}",
            frames.iter().map(|f| f.arrow).collect::<Vec<_>>()
        );
        assert!(frames[3].lean > 0.02, "lean never rises: {frames:?}");
        assert!(
            frames[10].lean < frames[5].lean,
            "lean never settles: {frames:?}"
        );
    }

    #[test]
    fn seed95_window_replays_exact() {
        assert_eq!(drive_window(), drive_window());
    }

    #[test]
    fn rejects_bad_input_without_mutation() {
        let mut r = Readable::new([0.0, 1.0]);
        let before = r.clone();
        assert_eq!(
            r.update(f32::NAN, 0.0, [0.0, 1.0]),
            Err(ReadableError::NonFiniteInput("mx"))
        );
        assert_eq!(
            r.update(0.0, f32::INFINITY, [0.0, 1.0]),
            Err(ReadableError::NonFiniteInput("mz"))
        );
        assert_eq!(
            r.update(0.0, 0.0, [f32::NAN, 0.0]),
            Err(ReadableError::NonFiniteInput("facing"))
        );
        assert_eq!(
            r.update(0.0, 0.0, [0.0, 0.0]),
            Err(ReadableError::BadFacing)
        );
        assert_eq!(
            r.update(MAX_SPEED + 1.0, 0.0, [0.0, 1.0]),
            Err(ReadableError::SpeedTooHigh(MAX_SPEED + 1.0))
        );
        assert_eq!(r, before, "rejected input mutates nothing");
        assert_eq!(slide_mm([f32::NAN, 0.0, 0.0], [0.0; 3]), u32::MAX);
        assert!(!feet_apart([0.0, 0.0, 0.0], [0.01, 0.0, 0.0]));
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "oracle uses real sin/cos; production tick keeps + - * / plus sqrt"
    )]
    fn yaw_step_matches_sin_cos() {
        // Oracle: production uses `sin(d) ~ d`, `cos(d) ~ 1 - d^2/2`.
        // Real `sin`/`cos` appear here only, never in the tick.
        let mut worst_sin = 0.0f32;
        let mut worst_cos = 0.0f32;
        let mut d = -ARROW_MAX_TURN;
        while d <= ARROW_MAX_TURN {
            let approx_sin = d;
            let approx_cos = 1.0 - d * d * 0.5;
            worst_sin = worst_sin.max((approx_sin - d.sin()).abs());
            worst_cos = worst_cos.max((approx_cos - d.cos()).abs());
            d += 0.005;
        }
        assert!(worst_sin < 2.9e-4, "sin approx drifts: {worst_sin}");
        assert!(worst_cos < 8.7e-6, "cos approx drifts: {worst_cos}");
    }

    #[test]
    fn update_allocates_nothing_per_tick() {
        let mut r = Readable::new([0.0, 1.0]);
        let count = measure(|| {
            for _ in 0..60 {
                let _ = r.update(1.5, 0.2, [0.0, 1.0]);
            }
        })
        .1;
        assert_eq!(count.calls, 0, "readable tick allocates: {count:?}");
    }
}
