//! photo_mode: the duel's photo mode — freeze, fly, lens, roll, still (SW141).
//!
//! One state machine over the existing presentation camera
//! ([`DirectorParams`], [`CameraMode::Free`]): `Off -> Entered -> Shooting ->
//! Restored`. Entering snapshots every knob it will touch and holds the sim
//! tick the caller was on; while the mode is not `Off` the host steps no tick
//! (`sim_paused`), so the frozen fight is the last tick's fight. Flight is
//! damped and bounded inside one sphere (the arena radius), the lens is a
//! 24..=200 mm focal length mapped to the camera's `fov_y_rad`, the roll is a
//! lens roll, and the still is the frame at [`SUPERSAMPLE`] times the base
//! size, capped at a maximum dimension (a potato tier captures 1x).
//!
//! Leaving restores the snapshot field for field, so the camera returns
//! byte-equal to the pre-enter pose. Presentation only: no clock, no wall
//! time, no random call, and the simulation never reads this module, so the
//! golden hash cannot move. Units: meters, radians, millimeters, EV, pixels.

use crate::camera::{free_cam_step, CameraError, CameraMode, DirectorParams, Vec3};

/// Supersample factor of a still (the donor's `SUPERSAMPLE_FACTOR`).
pub const SUPERSAMPLE: f32 = 2.0;
/// Widest focal length, millimeters (a 24 mm lens).
pub const MIN_FOCAL_MM: f32 = 24.0;
/// Longest focal length, millimeters (a 200 mm lens).
pub const MAX_FOCAL_MM: f32 = 200.0;
/// Full-frame sensor height the focal length is mapped through, millimeters.
pub const SENSOR_HEIGHT_MM: f32 = 24.0;
/// Largest roll either way, radians: a quarter turn, never upside down.
pub const MAX_ROLL_RAD: f32 = core::f32::consts::FRAC_PI_2;
/// Exposure compensation range, EV stops either way.
pub const MAX_EXPOSURE_EV: f32 = 5.0;
/// Default capture cap: an 8.3 MP still (3840x2160 from 1920x1080 at 2x).
pub const MAX_CAPTURE_DIM: u32 = 3840;
/// Fixed flight step, seconds: one photo frame at 60 Hz.
pub const FLIGHT_DT_S: f32 = 1.0 / 60.0;
/// Velocity kept per flight step; the rest is the damped pull toward the wish.
pub const FLIGHT_DAMPING: f32 = 0.85;
/// Acceleration a full wish adds per step, meters per second squared.
pub const FLIGHT_ACCEL_MPS2: f32 = 24.0;
/// Fastest the eye flies, meters per second.
pub const FLIGHT_MAX_MPS: f32 = 12.0;

/// Where the fight is: a sphere the photo eye may fly inside.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PhotoBounds {
    /// Sphere center (the arena's middle), meters.
    pub center_m: Vec3,
    /// Sphere radius (half the arena extent), meters.
    pub radius_m: f32,
}

impl PhotoBounds {
    /// The arena's own floor: a sphere over the middle of a square arena.
    #[must_use]
    pub fn arena(half_extent_m: f32) -> Self {
        Self {
            center_m: Vec3::new(0.0, 2.0, 0.0),
            radius_m: half_extent_m.max(1.0),
        }
    }

    /// True when the eye is inside the sphere.
    #[must_use]
    pub fn holds(&self, eye: Vec3) -> bool {
        let d = eye - self.center_m;
        d.len_sq() <= self.radius_m * self.radius_m
    }
}

/// Photo mode state; only [`PhotoState::Off`] lets the host step the sim.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PhotoState {
    /// Playing: no photo camera, the sim steps.
    #[default]
    Off,
    /// Frozen and flying: the sim holds its tick.
    Entered,
    /// The still is being taken; the sim still holds.
    Shooting,
    /// Leaving: knobs restored, the sim may step again.
    Restored,
}

impl PhotoState {
    /// True while the sim must hold its tick.
    #[must_use]
    pub fn freezes_sim(self) -> bool {
        matches!(self, PhotoState::Entered | PhotoState::Shooting)
    }

    /// True once the mode gave the camera back.
    #[must_use]
    pub fn released(self) -> bool {
        matches!(self, PhotoState::Off | PhotoState::Restored)
    }

    /// The status word a HUD or a sidecar prints.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PhotoState::Off => "off",
            PhotoState::Entered => "entered",
            PhotoState::Shooting => "shooting",
            PhotoState::Restored => "restored",
        }
    }
}

/// Failures a photo request can hit, each naming the value at fault.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PhotoError {
    /// The mode was already on when asked to enter.
    AlreadyOn,
    /// The mode was off when a photo-only call arrived.
    NotOn,
    /// The focal length is outside [`MIN_FOCAL_MM`]..=[`MAX_FOCAL_MM`].
    BadFocalMm,
    /// The roll is outside `-MAX_ROLL_RAD..=MAX_ROLL_RAD`.
    BadRoll,
    /// The exposure is outside `-MAX_EXPOSURE_EV..=MAX_EXPOSURE_EV`.
    BadExposure,
    /// A capture size, cap or base was zero or over the cap.
    BadCaptureSize,
    /// The camera math refused a step (see [`CameraError`]).
    Camera(CameraError),
}

impl From<CameraError> for PhotoError {
    fn from(error: CameraError) -> Self {
        PhotoError::Camera(error)
    }
}

impl core::fmt::Display for PhotoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PhotoError::AlreadyOn => write!(f, "photo: the mode is already on"),
            PhotoError::NotOn => write!(f, "photo: the mode is off; enter it first"),
            PhotoError::BadFocalMm => write!(
                f,
                "photo: focal length outside {MIN_FOCAL_MM}..={MAX_FOCAL_MM} mm"
            ),
            PhotoError::BadRoll => write!(f, "photo: roll outside +-{MAX_ROLL_RAD} rad"),
            PhotoError::BadExposure => {
                write!(f, "photo: exposure outside +-{MAX_EXPOSURE_EV} EV")
            }
            PhotoError::BadCaptureSize => write!(f, "photo: capture size or cap is zero/too big"),
            PhotoError::Write(why) => write!(f, "photo: cannot write the still ({why})"),
            PhotoError::Camera(error) => write!(f, "photo: camera refused the step ({error})"),
        }
    }
}

impl std::error::Error for PhotoError {}

/// What a still asked for: the pixel size, the sidecar JSON and the tick.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CapturePlan {
    /// Capture width in pixels (supersampled).
    pub width: u32,
    /// Capture height in pixels (supersampled).
    pub height: u32,
    /// The sidecar the still is saved next to: eye, target, fov, grade, tier,
    /// tick.
    pub sidecar: String,
}

/// Where a still went: the PNG and its JSON sidecar.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StillFiles {
    /// The written PNG.
    pub png: std::path::PathBuf,
    /// The sidecar beside it.
    pub sidecar: std::path::PathBuf,
}

/// Live photo session: the frozen tick, the flight velocity, the lens and the
/// knobs to put back.
#[derive(Clone, PartialEq, Debug)]
pub struct PhotoMode {
    state: PhotoState,
    /// Pre-enter camera, restored field for field on the way out.
    snapshot: Option<DirectorParams>,
    /// Camera mode to restore (`Director` in the arena).
    snapshot_mode: CameraMode,
    held_tick: u32,
    bounds: PhotoBounds,
    velocity_mps: Vec3,
    focal_mm: f32,
    roll_rad: f32,
    exposure_ev: f32,
    /// Pixels the base frame renders at before the supersample.
    base: (u32, u32),
    cap: u32,
}

impl Default for PhotoMode {
    fn default() -> Self {
        Self::new()
    }
}

impl PhotoMode {
    /// Off, with the arena defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: PhotoState::Off,
            snapshot: None,
            snapshot_mode: CameraMode::Director,
            held_tick: 0,
            bounds: PhotoBounds::arena(20.0),
            velocity_mps: Vec3::new(0.0, 0.0, 0.0),
            focal_mm: 50.0,
            roll_rad: 0.0,
            exposure_ev: 0.0,
            base: (1920, 1080),
            cap: MAX_CAPTURE_DIM,
        }
    }

    /// Freeze at `tick`, move the camera to the free eye and open the mode.
    /// The pre-enter camera is snapshotted first, so [`exit`](Self::exit)
    /// restores it byte for byte.
    pub fn enter(
        &mut self,
        params: &mut DirectorParams,
        mode: CameraMode,
        tick: u32,
        bounds: PhotoBounds,
    ) -> Result<(), PhotoError> {
        if !self.state.released() {
            return Err(PhotoError::AlreadyOn);
        }
        params.validate()?;
        if !bounds.radius_m.is_finite() || bounds.radius_m <= 0.0 {
            return Err(PhotoError::BadCaptureSize);
        }
        self.snapshot = Some(*params);
        self.snapshot_mode = mode;
        self.held_tick = tick;
        self.bounds = bounds;
        self.velocity_mps = Vec3::new(0.0, 0.0, 0.0);
        self.state = PhotoState::Entered;
        self.apply_lens(params)
    }

    /// The frozen tick: entering at tick 640 keeps 640 however long the
    /// photographer flies.
    #[must_use]
    pub fn held_tick(&self) -> u32 {
        self.held_tick
    }

    /// True while the host must not step the simulation.
    #[must_use]
    pub fn sim_paused(&self) -> bool {
        self.state.freezes_sim()
    }

    /// Current state.
    #[must_use]
    pub fn state(&self) -> PhotoState {
        self.state
    }

    /// Focal length in millimeters, `24..=200`.
    #[must_use]
    pub fn focal_mm(&self) -> f32 {
        self.focal_mm
    }

    /// Roll in radians, `+-MAX_ROLL_RAD`.
    #[must_use]
    pub fn roll_rad(&self) -> f32 {
        self.roll_rad
    }

    /// Exposure compensation in EV.
    #[must_use]
    pub fn exposure_ev(&self) -> f32 {
        self.exposure_ev
    }

    /// One damped flight frame: `wish` is the six-axis stick in meters per
    /// second (`[strafe, up, forward]`), `look` the wanted `(yaw, pitch)` in
    /// radians. The eye is clamped inside [`PhotoBounds`], so a held stick
    /// cannot leave the arena. Call it once per photo frame.
    pub fn fly(
        &mut self,
        params: &mut DirectorParams,
        wish: [f32; 3],
        look: (f32, f32),
    ) -> Result<(), PhotoError> {
        self.require_on()?;
        let target = Vec3::new(wish[0], wish[1], wish[2]);
        if !target.is_finite() {
            return Err(PhotoError::Camera(CameraError::BadParam("photo_wish")));
        }
        let scale = FLIGHT_ACCEL_MPS2 * FLIGHT_DT_S;
        let mut velocity = self.velocity_mps * FLIGHT_DAMPING + target * scale;
        let speed = velocity.len();
        if speed > FLIGHT_MAX_MPS {
            velocity = velocity * (FLIGHT_MAX_MPS / speed);
        }
        self.velocity_mps = velocity;
        let step = velocity * FLIGHT_DT_S;
        free_cam_step(params, look, [step.x, step.y, step.z])?;
        let eye = self.bounded(params.free_eye_m);
        params.free_eye_m = eye;
        Ok(())
    }

    /// Set the lens: a 24 mm lens is wide, a 200 mm lens is tight. The
    /// camera's full vertical field of view follows
    /// `2 * atan(sensor_height / (2 * focal))`.
    pub fn set_focal_mm(
        &mut self,
        params: &mut DirectorParams,
        focal_mm: f32,
    ) -> Result<(), PhotoError> {
        self.require_on()?;
        if !focal_mm.is_finite() || !(MIN_FOCAL_MM..=MAX_FOCAL_MM).contains(&focal_mm) {
            return Err(PhotoError::BadFocalMm);
        }
        self.focal_mm = focal_mm;
        self.apply_lens(params)
    }

    /// Roll the lens, `-MAX_ROLL_RAD..=MAX_ROLL_RAD`; a roll is view state,
    /// never a camera record row.
    pub fn set_roll_rad(&mut self, roll_rad: f32) -> Result<(), PhotoError> {
        self.require_on()?;
        if !roll_rad.is_finite() || roll_rad.abs() > MAX_ROLL_RAD {
            return Err(PhotoError::BadRoll);
        }
        self.roll_rad = roll_rad;
        Ok(())
    }

    /// Exposure compensation into the post grade, `+-MAX_EXPOSURE_EV` stops.
    pub fn set_exposure_ev(&mut self, ev: f32) -> Result<(), PhotoError> {
        self.require_on()?;
        if !ev.is_finite() || ev.abs() > MAX_EXPOSURE_EV {
            return Err(PhotoError::BadExposure);
        }
        self.exposure_ev = ev;
        Ok(())
    }

    /// The grade row the still is developed with: exposure in EV, the roll and
    /// the tier, one line a HUD or a sidecar prints.
    #[must_use]
    pub fn grade_line(&self, potato: bool) -> String {
        format!(
            "exposure {:.2} EV, roll {:.3} rad, tier {}, supersample {:.1}",
            self.exposure_ev,
            self.roll_rad,
            if potato { "potato" } else { "full" },
            if potato { 1.0 } else { SUPERSAMPLE }
        )
    }

    /// The still's pixel size for a `base` frame at `cap` on the long edge:
    /// [`SUPERSAMPLE`] times, clamped so neither side passes `cap` (a potato
    /// tier captures 1x). `None` when the base or the cap is unusable.
    #[must_use]
    pub fn capture_size(base: (u32, u32), cap: u32, potato: bool) -> Option<(u32, u32)> {
        let (w, h) = base;
        if w == 0 || h == 0 || cap == 0 {
            return None;
        }
        let factor = if potato { 1 } else { 2 };
        let (mut out_w, mut out_h) = (w.saturating_mul(factor), h.saturating_mul(factor));
        let long = out_w.max(out_h);
        if long > cap {
            // Keep the aspect ratio: scale both sides by the same ratio.
            let cap_long = cap;
            out_w = out_w.saturating_mul(cap_long) / long;
            out_h = out_h.saturating_mul(cap_long) / long;
        }
        if out_w == 0 || out_h == 0 {
            return None;
        }
        Some((out_w, out_h))
    }

    /// Take the still: state goes `Shooting`, the plan carries the supersampled
    /// size and the JSON sidecar naming where it was taken. The base size and
    /// the cap are the ones [`set_capture`](Self::set_capture) stored.
    pub fn shoot(
        &mut self,
        params: &DirectorParams,
        target: Vec3,
        potato: bool,
    ) -> Result<CapturePlan, PhotoError> {
        self.require_on()?;
        let (width, height) =
            Self::capture_size(self.base, self.cap, potato).ok_or(PhotoError::BadCaptureSize)?;
        self.state = PhotoState::Shooting;
        let sidecar = self.sidecar(params, target, potato);
        Ok(CapturePlan {
            width,
            height,
            sidecar,
        })
    }

    /// The base render size and the long-edge cap a still uses.
    pub fn set_capture(&mut self, base: (u32, u32), cap: u32) -> Result<(), PhotoError> {
        self.require_on()?;
        if base.0 == 0 || base.1 == 0 || cap == 0 {
            return Err(PhotoError::BadCaptureSize);
        }
        if Self::capture_size(base, cap, false).is_none() {
            return Err(PhotoError::BadCaptureSize);
        }
        self.base = base;
        self.cap = cap;
        Ok(())
    }

    /// The JSON sidecar: where the still was taken (eye, target, fov, grade,
    /// tier, tick), one object, fixed key order, no clock.
    #[must_use]
    pub fn sidecar(&self, params: &DirectorParams, target: Vec3, potato: bool) -> String {
        let eye = params.free_eye_m;
        format!(
            "{{\"tool\":\"photo.shoot\",\"eye\":[{:.4},{:.4},{:.4}],\"target\":[{:.4},{:.4},{:.4}],\
             \"yaw_rad\":{:.4},\"pitch_rad\":{:.4},\"fov_y_rad\":{:.5},\"roll_rad\":{:.4},\
             \"exposure_ev\":{:.2},\"focal_mm\":{:.1},\"tier\":\"{}\",\"supersample\":{:.1},\
             \"tick\":{}}}",
            eye.x,
            eye.y,
            eye.z,
            target.x,
            target.y,
            target.z,
            params.free_yaw_rad,
            params.free_pitch_rad,
            params.fov_y_rad,
            self.roll_rad,
            self.exposure_ev,
            self.focal_mm,
            if potato { "potato" } else { "full" },
            if potato { 1.0 } else { SUPERSAMPLE },
            self.held_tick
        )
    }

    /// Write the still: the PNG bytes the host rendered plus the plan's JSON
    /// sidecar, named by the parked tick and the lens, so writing the same
    /// still twice overwrites one file instead of doubling it. Returns both
    /// paths.
    pub fn write_still(
        &mut self,
        plan: &CapturePlan,
        png: &[u8],
        dir: &std::path::Path,
    ) -> Result<StillFiles, PhotoError> {
        self.require_on()?;
        if png.is_empty() {
            return Err(PhotoError::BadCaptureSize);
        }
        std::fs::create_dir_all(dir)
            .map_err(|error| PhotoError::Write(format!("{}: {error}", dir.display())))?;
        let stem = format!("photo-t{}-{:.0}mm", self.held_tick, self.focal_mm);
        let png_path = dir.join(format!("{stem}.png"));
        let json_path = dir.join(format!("{stem}.json"));
        std::fs::write(&png_path, png)
            .map_err(|error| PhotoError::Write(format!("{}: {error}", png_path.display())))?;
        std::fs::write(&json_path, plan.sidecar.as_bytes())
            .map_err(|error| PhotoError::Write(format!("{}: {error}", json_path.display())))?;
        Ok(StillFiles {
            png: png_path,
            sidecar: json_path,
        })
    }

    /// Give the camera back: the snapshot is written over every field the mode
    /// touched, so the pose is byte-equal to the pre-enter one. Returns the
    /// camera mode that was live before entering.
    pub fn exit(&mut self, params: &mut DirectorParams) -> Result<CameraMode, PhotoError> {
        self.require_on()?;
        let snapshot = self.snapshot.ok_or(PhotoError::NotOn)?;
        *params = snapshot;
        self.state = PhotoState::Restored;
        self.snapshot = None;
        self.velocity_mps = Vec3::new(0.0, 0.0, 0.0);
        self.roll_rad = 0.0;
        self.exposure_ev = 0.0;
        Ok(self.snapshot_mode)
    }

    /// Write the lens into the camera's `fov_y_rad`.
    fn apply_lens(&mut self, params: &mut DirectorParams) -> Result<(), PhotoError> {
        let fov = 2.0 * (SENSOR_HEIGHT_MM / (2.0 * self.focal_mm)).atan();
        if !fov.is_finite() || fov <= 0.0 || fov >= core::f32::consts::PI {
            return Err(PhotoError::BadFocalMm);
        }
        params.fov_y_rad = fov;
        Ok(())
    }

    /// Clamp an eye to the bounds sphere, keeping its direction.
    fn bounded(&self, eye: Vec3) -> Vec3 {
        let d = eye - self.bounds.center_m;
        let dist = d.len();
        if dist <= self.bounds.radius_m || dist <= f32::EPSILON {
            return eye;
        }
        self.bounds.center_m + d * (self.bounds.radius_m / dist)
    }

    fn require_on(&self) -> Result<(), PhotoError> {
        if self.state.freezes_sim() {
            Ok(())
        } else {
            Err(PhotoError::NotOn)
        }
    }
}

#[cfg(test)]
#[path = "photo_mode_tests.rs"]
mod tests;
