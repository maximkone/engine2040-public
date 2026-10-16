//! fps_view: first-person eye pose, presentation side.
//! Owner: engine-game. Units: mm in, m out for the camera feed; yaw and pitch in rad.
//! Determinism: presentation only, never hashed; same deltas give same bytes twice.
use crate::camera::CameraError;
use std::f32::consts::{FRAC_PI_2, PI, TAU};
/// Mouse sensitivity.
pub const SENSITIVITY: f32 = 0.001;
/// Pitch pad from the pole.
pub const ANGLE_EPSILON: f32 = 0.001953125;
/// Eye height over feet, mm.
pub const EYE_HEIGHT_MM: i32 = 1600;
/// Interp scale: permille 0..=1000 maps to alpha 0..=1.
pub const INTERP_SCALE: u32 = 1000;
/// Look state: yaw and pitch radians.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LookState {
    pub yaw_rad: f32,
    pub pitch_rad: f32,
}
impl LookState {
    /// Fresh look facing forward.
    pub fn neutral() -> Self {
        Self {
            yaw_rad: 0.0,
            pitch_rad: 0.0,
        }
    }
    /// Fold one mouse delta into yaw and pitch; pitch clamps, yaw wraps to pi range.
    pub fn apply_mouse(&mut self, dx_px: f32, dy_px: f32) {
        if !dx_px.is_finite() || !dy_px.is_finite() {
            return;
        }
        let limit = FRAC_PI_2 - ANGLE_EPSILON;
        self.pitch_rad = (self.pitch_rad - dy_px * SENSITIVITY).clamp(-limit, limit);
        self.yaw_rad -= dx_px * SENSITIVITY;
        if self.yaw_rad.abs() > PI {
            self.yaw_rad = self.yaw_rad.rem_euclid(TAU);
            if self.yaw_rad > PI {
                self.yaw_rad -= TAU;
            }
        }
    }
}
/// Logical frame: current plus previous feet mm with look.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LogicalFrame {
    pub pos_mm: [i32; 3],
    pub prev_mm: [i32; 3],
    pub look: LookState,
}
/// Render eye: interpolated feet plus height, with look attached.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RenderEye {
    pub eye_mm: [f64; 3],
    pub yaw_rad: f32,
    pub pitch_rad: f32,
}
fn lerp_lane(a: i32, b: i32, alpha_q: u32) -> f64 {
    let t = f64::from(alpha_q.min(INTERP_SCALE)) / f64::from(INTERP_SCALE);
    f64::from(a) + (f64::from(b) - f64::from(a)) * t
}
/// Build the render eye at overstep alpha permille; adds height over interpolated feet.
pub fn render_eye(frame: &LogicalFrame, alpha_q: u32, height_mm: i32) -> RenderEye {
    RenderEye {
        eye_mm: [
            lerp_lane(frame.prev_mm[0], frame.pos_mm[0], alpha_q),
            lerp_lane(frame.prev_mm[1], frame.pos_mm[1], alpha_q) + f64::from(height_mm),
            lerp_lane(frame.prev_mm[2], frame.pos_mm[2], alpha_q),
        ],
        yaw_rad: frame.look.yaw_rad,
        pitch_rad: frame.look.pitch_rad,
    }
}
/// Eye triple in meters for the render camera feed.
pub fn eye_m(eye: &RenderEye) -> [f64; 3] {
    [
        eye.eye_mm[0] / 1000.0,
        eye.eye_mm[1] / 1000.0,
        eye.eye_mm[2] / 1000.0,
    ]
}
/// First-person eye over one fighter body: eye sits `height_mm` over `pos_m` with yaw from facing.
/// Units: `pos_m` meters, `height_mm` mm in 1..=2500, yaw rad; pitch is 0.
/// Errors: `BadParam` naming `FV1` plus the bad field for non-finite input or bad height.
/// Determinism: presentation only, never hashed; same inputs give same bytes twice; zero heap.
pub fn eye_for_fighter(
    pos_m: [f32; 3],
    facing_yaw_rad: f32,
    height_mm: i32,
) -> Result<RenderEye, CameraError> {
    if !pos_m[0].is_finite() || !pos_m[1].is_finite() || !pos_m[2].is_finite() {
        return Err(CameraError::BadParam("FV1 pos_m"));
    }
    if !facing_yaw_rad.is_finite() {
        return Err(CameraError::BadParam("FV1 facing_yaw_rad"));
    }
    if !(1..=2500).contains(&height_mm) {
        return Err(CameraError::BadParam("FV1 height_mm"));
    }
    Ok(RenderEye {
        eye_mm: [
            f64::from(pos_m[0]) * 1000.0,
            f64::from(pos_m[1]) * 1000.0 + f64::from(height_mm),
            f64::from(pos_m[2]) * 1000.0,
        ],
        yaw_rad: facing_yaw_rad,
        pitch_rad: 0.0,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fps_look_clamps_and_wraps() {
        let mut look = LookState::neutral();
        look.apply_mouse(0.0, -100000.0);
        let top = FRAC_PI_2 - ANGLE_EPSILON;
        assert!(
            (look.pitch_rad - top).abs() < 0.0001,
            "pitch not clamped: {:?}",
            look.pitch_rad
        );
        look.apply_mouse(f32::NAN, 0.0);
        assert!(
            (look.pitch_rad - top).abs() < 0.0001,
            "nan moved pitch: {:?}",
            look.pitch_rad
        );
        look.apply_mouse(20000.0, 0.0);
        assert!(
            look.yaw_rad >= -PI && look.yaw_rad <= PI,
            "yaw not wrapped: {:?}",
            look.yaw_rad
        );
        let frame = LogicalFrame {
            pos_mm: [1000, 0, 0],
            prev_mm: [0, 0, 0],
            look,
        };
        let e0 = render_eye(&frame, 0, EYE_HEIGHT_MM);
        let e1 = render_eye(&frame, 1000, EYE_HEIGHT_MM);
        assert_eq!(e0.eye_mm[0], 0.0, "alpha 0 missed prev");
        assert_eq!(e1.eye_mm[0], 1000.0, "alpha max missed current");
        assert_eq!(
            render_eye(&frame, 500, EYE_HEIGHT_MM),
            render_eye(&frame, 500, EYE_HEIGHT_MM),
            "eye replay diverged"
        );
    }
    #[test]
    fn eye_for_fighter_puts_eye_at_head_height() {
        let eye = eye_for_fighter([1.0, 2.0, 3.0], 0.75, EYE_HEIGHT_MM).expect("valid eye");
        assert_eq!(eye.eye_mm[0], 1000.0, "x missed pos");
        assert_eq!(
            eye.eye_mm[1],
            2000.0 + f64::from(EYE_HEIGHT_MM),
            "y missed height"
        );
        assert_eq!(eye.eye_mm[2], 3000.0, "z missed pos");
        assert_eq!(eye.yaw_rad, 0.75, "yaw missed facing");
        assert_eq!(eye.pitch_rad, 0.0, "pitch must be 0");
    }
    #[test]
    fn eye_for_fighter_refuses_bad_params() {
        for result in [
            eye_for_fighter([f32::NAN, 0.0, 0.0], 0.0, EYE_HEIGHT_MM),
            eye_for_fighter([0.0, 0.0, 0.0], f32::NAN, EYE_HEIGHT_MM),
            eye_for_fighter([0.0, 0.0, 0.0], 0.0, 0),
            eye_for_fighter([0.0, 0.0, 0.0], 0.0, 3000),
        ] {
            assert!(
                matches!(result, Err(CameraError::BadParam(row)) if row.contains("FV1")),
                "bad input accepted: {result:?}"
            );
        }
    }
    #[test]
    fn eye_for_fighter_replays_byte_exact_twice() {
        let first = eye_for_fighter([1.5, 0.5, -2.0], -1.2, 1600).expect("valid eye");
        let second = eye_for_fighter([1.5, 0.5, -2.0], -1.2, 1600).expect("valid eye");
        assert_eq!(first, second, "eye replay diverged");
    }
    #[test]
    fn eye_for_fighter_allocates_nothing() {
        let (_, probe) = engine_core::alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert_ne!(probe.calls, 0, "sanity: probe sees heap");
        let (eye, steady) = engine_core::alloc_probe::measure(|| {
            eye_for_fighter([1.0, 2.0, 3.0], 0.75, EYE_HEIGHT_MM).expect("valid eye")
        });
        assert_eq!(steady.calls, 0, "eye pose must not allocate");
        assert_eq!(eye.eye_mm[0], 1000.0, "sanity: eye moved");
    }
}
