//! SPF2 closed-form camera frame fit in integer cm.
//!
//! Purpose: place a camera eye on a named view ray so all eight corners of a
//! subject box stay inside `fill_pct` of the frame. Pure integer math over the
//! crate's frozen `sin_1k`/`cos_1k` table (half-degree steps by linear
//! interpolation), so the same request gives the same bytes on every machine:
//! no libm, no float. Consumer: the `scene.frame` door verb; until then the
//! unit tests are the caller.
//!
//! World: x east, y up, z north, lengths in cm, angles in whole degrees. A view
//! yaw is the azimuth of the EYE seen from the target (0 = on the +z side,
//! 90 = on the +x side); the returned camera yaw is the look direction
//! (eye yaw + 180) and the camera pitch is negative when it looks down.
//! Determinism: same inputs give the same [`FrameFit`]; box axis order is the
//! x, y, z order. Failure mode: a bad box or a range outside the table returns
//! a typed [`FrameError`], never a nearest guess.

use std::fmt;

use crate::index::Box3;
use crate::sense_text::{cos_1k, sin_1k};

/// Smallest eye distance in cm (a tiny box never puts the eye inside it).
pub const MIN_DISTANCE_CM: i32 = 50;
/// Direction fixed-point scale: one unit is `SCALE`.
const SCALE: i64 = 1 << 20;
/// Factor from the 1024-scale sine table to `SCALE`.
const FROM_1K: i64 = SCALE / 1024;

/// `sin(deg)` in fixed point for any whole degree.
fn sin_deg(deg: i32) -> i64 {
    i64::from(sin_1k(deg)) * FROM_1K
}

/// `cos(deg)` in fixed point for any whole degree.
fn cos_deg(deg: i32) -> i64 {
    i64::from(cos_1k(deg)) * FROM_1K
}

/// `sin(half_deg / 2 degrees)` in fixed point for any whole half-degree step:
/// linear interpolation between the two neighbouring table degrees, accurate to
/// about 1e-5 so an odd fov (27.5 degrees) keeps its half.
fn sin_half(half_deg: i32) -> i64 {
    let deg = half_deg.div_euclid(2);
    let rem = i64::from(half_deg.rem_euclid(2));
    let near = (2 - rem) * i64::from(sin_1k(deg)) + rem * i64::from(sin_1k(deg + 1));
    near * FROM_1K / 2
}

/// `cos(half_deg / 2 degrees)` in fixed point.
fn cos_half(half_deg: i32) -> i64 {
    sin_half(half_deg + 180)
}

/// `tan(fov / 2)` in fixed point; fov is capped at 120 so the cosine is positive.
fn tan_half_fov(fov_deg: i32) -> i64 {
    let cos = cos_half(fov_deg);
    if cos == 0 {
        return i64::MAX / 2;
    }
    sin_half(fov_deg) * SCALE / cos
}

/// Camera basis for a look yaw and pitch (whole degrees, pitch up positive):
/// forward, right, up, each a `SCALE`-fixed unit vector.
struct Basis {
    forward: [i64; 3],
    right: [i64; 3],
    up: [i64; 3],
}

fn basis(look_yaw: i32, look_pitch: i32) -> Basis {
    let (sy, cy) = (sin_deg(look_yaw), cos_deg(look_yaw));
    let (sp, cp) = (sin_deg(look_pitch), cos_deg(look_pitch));
    let forward = [sy * cp / SCALE, sp, cy * cp / SCALE];
    let up = [-sy * sp / SCALE, cp, -cy * sp / SCALE];
    // forward and up are orthogonal, so their cross product is already unit
    // up to the table's own rounding.
    let right = [
        (up[1] * forward[2] - up[2] * forward[1]) / SCALE,
        (up[2] * forward[0] - up[0] * forward[2]) / SCALE,
        (up[0] * forward[1] - up[1] * forward[0]) / SCALE,
    ];
    Basis { forward, right, up }
}

/// Round-half-away-from-zero of a 2*cm mid-point back to whole cm.
fn mid_cm(min: i32, max: i32) -> i32 {
    let sum = i64::from(min) + i64::from(max);
    let half = if sum >= 0 {
        (sum + 1) / 2
    } else {
        (sum - 1) / 2
    };
    i32::try_from(half).unwrap_or(i32::MAX)
}

/// Signed contribution `sum(s_i * d_i * v_i)` for one box corner, in cm * `SCALE`.
fn axis_dot(sign: [i64; 3], extent: [i64; 3], v: [i64; 3]) -> i128 {
    sign.iter()
        .zip(extent)
        .zip(v)
        .map(|((&s, d), val)| i128::from(s) * i128::from(d) * i128::from(val))
        .sum()
}

/// Ceiling of `num / den` for a positive `den` and any signed `num`.
fn ceil_div(num: i128, den: i128) -> i128 {
    debug_assert!(den > 0);
    let floor = num.div_euclid(den);
    floor + i128::from(num.rem_euclid(den) != 0)
}

/// Named view; the angle pairs are the shared `CameraShot` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Eye yaw 35, pitch 30.
    ThreeQuarter,
    /// Eye yaw 0, pitch 10.
    Front,
    /// Eye yaw 180, pitch 10.
    Back,
    /// Eye yaw 90, pitch 10.
    Side,
    /// Eye yaw 0, pitch 89.
    TopDown,
    /// Behind the target facing yaw, pitch 20.
    ThirdPerson,
    /// Eye yaw and pitch from the request.
    Orbit,
}

impl View {
    /// Every view, in report order.
    pub const ALL: [Self; 7] = [
        Self::ThreeQuarter,
        Self::Front,
        Self::Back,
        Self::Side,
        Self::TopDown,
        Self::ThirdPerson,
        Self::Orbit,
    ];

    /// Door word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ThreeQuarter => "three_quarter",
            Self::Front => "front",
            Self::Back => "back",
            Self::Side => "side",
            Self::TopDown => "top_down",
            Self::ThirdPerson => "third_person",
            Self::Orbit => "orbit",
        }
    }

    /// Eye yaw and pitch for this view, the one owner of the `CameraShot`
    /// angle pairs; `yaw_deg` faces the target for [`View::ThirdPerson`] and
    /// is the eye azimuth for [`View::Orbit`].
    pub fn eye_angles(self, yaw_deg: i32, pitch_deg: i32) -> (i32, i32) {
        match self {
            Self::ThreeQuarter => (35, 30),
            Self::Front => (0, 10),
            Self::Back => (180, 10),
            Self::Side => (90, 10),
            Self::TopDown => (0, 89),
            Self::ThirdPerson => (yaw_deg + 180, 20),
            Self::Orbit => (yaw_deg, pitch_deg),
        }
    }
}

/// Typed refusal of a frame request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// `max` is below `min` on some axis.
    EmptyBox,
    /// A number is outside its legal range; names the field and the range.
    OutOfRange(&'static str),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyBox => write!(f, "frame: box max is below min; pass min and max in cm"),
            Self::OutOfRange(what) => write!(f, "frame: {what}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Camera pose that frames a box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameFit {
    /// Eye position in cm.
    pub eye_cm: [i32; 3],
    /// Box centre in cm (the point looked at).
    pub look_at_cm: [i32; 3],
    /// Eye to centre distance in cm.
    pub distance_cm: i32,
    /// Look yaw in degrees (eye yaw + 180), 0..=359.
    pub yaw_deg: i32,
    /// Look pitch in degrees, negative when looking down.
    pub pitch_deg: i32,
}

/// Refuse a request whose numbers fall outside the frozen table ranges.
fn check_ranges(
    fov_deg: i32,
    aspect_pct: i32,
    fill_pct: i32,
    pitch_deg: i32,
) -> Result<(), FrameError> {
    if !(20..=120).contains(&fov_deg) {
        return Err(FrameError::OutOfRange("fov_deg must be 20..=120"));
    }
    if !(50..=300).contains(&aspect_pct) {
        return Err(FrameError::OutOfRange("aspect_pct must be 50..=300"));
    }
    if !(10..=100).contains(&fill_pct) {
        return Err(FrameError::OutOfRange("fill_pct must be 10..=100"));
    }
    if !(-89..=89).contains(&pitch_deg) {
        return Err(FrameError::OutOfRange("pitch_deg must be -89..=89"));
    }
    Ok(())
}

/// Nearest eye on the view ray from the box centre that keeps all eight
/// corners inside `fill_pct` of the frame on both axes (closed form: the
/// largest of sixteen per-corner distances, never under [`MIN_DISTANCE_CM`]).
pub fn frame_fit(
    box_cm: &Box3,
    view: View,
    fov_deg: i32,
    aspect_pct: i32,
    fill_pct: i32,
    yaw_deg: i32,
    pitch_deg: i32,
) -> Result<FrameFit, FrameError> {
    if box_cm.max_x < box_cm.min_x || box_cm.max_y < box_cm.min_y || box_cm.max_z < box_cm.min_z {
        return Err(FrameError::EmptyBox);
    }
    check_ranges(fov_deg, aspect_pct, fill_pct, pitch_deg)?;
    let (eye_yaw, eye_pitch) = view.eye_angles(yaw_deg, pitch_deg);
    let look_yaw = (eye_yaw + 180).rem_euclid(360);
    let look_pitch = -eye_pitch;
    let b = basis(look_yaw, look_pitch);
    let tan = tan_half_fov(fov_deg);
    let ty = tan * i64::from(fill_pct) / 100;
    let tx = ty * i64::from(aspect_pct) / 100;
    let extent = [
        i64::from(box_cm.max_x) - i64::from(box_cm.min_x),
        i64::from(box_cm.max_y) - i64::from(box_cm.min_y),
        i64::from(box_cm.max_z) - i64::from(box_cm.min_z),
    ];
    let mut dist = i64::from(MIN_DISTANCE_CM);
    for mask in 0u8..8 {
        let sign = [
            if mask & 1 == 0 { -1 } else { 1 },
            if mask & 2 == 0 { -1 } else { 1 },
            if mask & 4 == 0 { -1 } else { 1 },
        ];
        let forward_dot = axis_dot(sign, extent, b.forward);
        let right_dot = axis_dot(sign, extent, b.right);
        let up_dot = axis_dot(sign, extent, b.up);
        // A corner at offset o (half the extent, signed) needs
        //   dist >= |o . right| / tx - o . forward
        // and the same on up/ty. In fixed point o . right = right_dot / (2 * SCALE),
        // so multiplying by 2 * SCALE gives this numerator:
        let need_right = right_dot.abs() * i128::from(SCALE) / i128::from(tx);
        let need_up = up_dot.abs() * i128::from(SCALE) / i128::from(ty);
        let numerator = need_right.max(need_up) - forward_dot;
        let denom = 2 * i128::from(SCALE);
        let each = i64::try_from(ceil_div(numerator, denom)).unwrap_or(i64::MAX);
        dist = dist.max(each);
    }
    let centre = [
        mid_cm(box_cm.min_x, box_cm.max_x),
        mid_cm(box_cm.min_y, box_cm.max_y),
        mid_cm(box_cm.min_z, box_cm.max_z),
    ];
    let mut eye = [0i32; 3];
    for ((slot, base), forward) in eye.iter_mut().zip(centre).zip(b.forward) {
        let shift = (forward * dist).div_euclid(SCALE);
        *slot = base - i32::try_from(shift).unwrap_or(i32::MAX);
    }
    Ok(FrameFit {
        eye_cm: eye,
        look_at_cm: centre,
        distance_cm: i32::try_from(dist).unwrap_or(i32::MAX),
        yaw_deg: look_yaw,
        pitch_deg: look_pitch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUBJECT: Box3 = Box3 {
        id: 1,
        min_x: -100,
        max_x: 100,
        min_y: 0,
        max_y: 180,
        min_z: -50,
        max_z: 50,
    };

    /// Independent projection of the returned pose against the FULL frame
    /// (no fill factor): the largest axis use of any corner, in permille.
    fn use_permille(fit: &FrameFit, fov_deg: i32, aspect_pct: i32) -> i128 {
        let b = basis(fit.yaw_deg, fit.pitch_deg);
        let tan = tan_half_fov(fov_deg);
        let tx = tan * i64::from(aspect_pct) / 100;
        let ty = tan;
        let mut worst = 0i128;
        for mask in 0u8..8 {
            let corner = [
                if mask & 1 == 0 {
                    SUBJECT.min_x
                } else {
                    SUBJECT.max_x
                },
                if mask & 2 == 0 {
                    SUBJECT.min_y
                } else {
                    SUBJECT.max_y
                },
                if mask & 4 == 0 {
                    SUBJECT.min_z
                } else {
                    SUBJECT.max_z
                },
            ];
            let rel = [
                i128::from(corner[0]) - i128::from(fit.eye_cm[0]),
                i128::from(corner[1]) - i128::from(fit.eye_cm[1]),
                i128::from(corner[2]) - i128::from(fit.eye_cm[2]),
            ];
            // Keep the raw cm * SCALE sums and divide once, so no intermediate
            // truncation moves the measured fill.
            let depth = rel
                .iter()
                .zip(b.forward)
                .map(|(&r, f)| r * i128::from(f))
                .sum::<i128>();
            assert!(depth > 0, "corner behind the eye");
            let lateral = |v: [i64; 3]| -> i128 {
                rel.iter()
                    .zip(v)
                    .map(|(&r, f)| r * i128::from(f))
                    .sum::<i128>()
                    .abs()
            };
            let share_r = lateral(b.right) * 1000 * i128::from(SCALE) / (depth * i128::from(tx));
            let share_u = lateral(b.up) * 1000 * i128::from(SCALE) / (depth * i128::from(ty));
            worst = worst.max(share_r.max(share_u));
        }
        worst
    }

    #[test]
    fn frame_fit_corners_inside_and_use_matches_fill() {
        for view in View::ALL {
            for fov in [55, 60, 65] {
                for aspect in [178, 133] {
                    let fit = frame_fit(&SUBJECT, view, fov, aspect, 85, 20, 25).expect("fits");
                    let used = use_permille(&fit, fov, aspect);
                    assert!(
                        (used - 850).abs() <= 10,
                        "{} fov {fov} aspect {aspect}: use {used}",
                        view.as_str()
                    );
                    assert_eq!(fit.look_at_cm, [0, 90, 0]);
                    assert_eq!(
                        fit,
                        frame_fit(&SUBJECT, view, fov, aspect, 85, 20, 25).expect("fits"),
                        "same bytes twice"
                    );
                }
            }
        }
    }

    #[test]
    fn frame_fit_top_down_and_third_person_geometry() {
        let top = frame_fit(&SUBJECT, View::TopDown, 60, 178, 85, 20, 25).expect("fits");
        assert!(
            top.eye_cm[1] > SUBJECT.max_y && top.pitch_deg == -89,
            "{top:?}"
        );
        // The target faces yaw 20; the eye must sit behind that facing.
        let fit = frame_fit(&SUBJECT, View::ThirdPerson, 60, 178, 85, 20, 25).expect("fits");
        let face = [sin_deg(20), cos_deg(20)];
        let behind = i128::from(fit.eye_cm[0]) * i128::from(face[0])
            + i128::from(fit.eye_cm[2]) * i128::from(face[1]);
        assert!(behind < 0, "eye is in front of the target: {fit:?}");
    }

    #[test]
    fn frame_fit_refuses_bad_input() {
        let flat = Box3 {
            id: 1,
            min_x: 5,
            max_x: 0,
            min_y: 0,
            max_y: 1,
            min_z: 0,
            max_z: 1,
        };
        assert_eq!(
            frame_fit(&flat, View::Front, 60, 178, 85, 0, 0),
            Err(FrameError::EmptyBox)
        );
        assert!(frame_fit(&SUBJECT, View::Front, 5, 178, 85, 0, 0).is_err());
        assert!(frame_fit(&SUBJECT, View::Front, 60, 178, 85, 0, 120).is_err());
        let tiny = Box3 {
            id: 2,
            min_x: 0,
            max_x: 1,
            min_y: 0,
            max_y: 1,
            min_z: 0,
            max_z: 1,
        };
        assert!(
            frame_fit(&tiny, View::Front, 60, 178, 85, 0, 0)
                .expect("fits")
                .distance_cm
                >= MIN_DISTANCE_CM
        );
    }
}
