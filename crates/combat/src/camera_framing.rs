//! camera_framing: third-person melee framing as pure integer functions (GP14).
//!
//! Principle (UNVERIFIED for 3D until a playtest says so): in a duel the
//! camera sits behind the attacker's shoulder, pulled back far enough to hold
//! both fighters and sideways far enough that the attacker's body does not
//! hide the defender. This module computes that pose from two ground
//! positions and reports a silhouette readability number the playtest can
//! gate on. Ideas only, no code taken from any source.
//!
//! Owner: engine-combat. Consumer (caller must hook in): the pair framing in
//! `crates/game/src/camera.rs` (`frame_pair` and the two-fighter case of
//! `frame_group`) would call [`frame_melee`](crate::camera_framing::frame_melee) and convert millimeters to
//! meters. Units: millimeters on a ground plane `[x, z]`, y up; angles as
//! tangent permille. Determinism: integer math only, no state, no clock, so
//! the pose at tick N depends on tick N's positions alone (no follow lag).
//! Positions stay under 1e9 mm (a thousand kilometers). Errors: none; coincident fighters fall back to a +z facing. Budget: a few
//! dozen integer ops, no heap.

/// Camera eye height above the ground in millimeters.
pub const EYE_HEIGHT_MM: i32 = 1900;
/// Look-at height in millimeters (chest level).
pub const LOOK_HEIGHT_MM: i32 = 1200;
/// Silhouette half width of a standing fighter in millimeters.
pub const SILHOUETTE_HALF_MM: i64 = 350;
/// Nearest boom length behind the attacker in millimeters.
pub const BACK_MIN_MM: i64 = 1800;
/// Farthest boom length behind the attacker in millimeters.
pub const BACK_MAX_MM: i64 = 4500;
/// Smallest shoulder offset to the side in millimeters.
pub const SIDE_MIN_MM: i64 = 800;
/// Largest shoulder offset to the side in millimeters.
pub const SIDE_MAX_MM: i64 = 2000;
/// How far along attacker-to-defender the camera looks, in permille.
pub const LOOK_BIAS_PERMILLE: i64 = 600;
/// Distances under this count as a clinch and use it instead, in millimeters.
pub const MIN_DIST_MM: i64 = 200;
/// Readability at or above this permille counts as a readable pose (80%).
pub const READABLE_PERMILLE: u32 = 800;

/// Which shoulder the camera sits over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shoulder {
    /// Camera to the attacker's left, looking along attacker to defender.
    Left,
    /// Camera to the attacker's right.
    Right,
}

/// A framing result for one tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Framing {
    /// Camera eye `[x, y, z]` in millimeters.
    pub eye_mm: [i32; 3],
    /// Look-at point `[x, y, z]` in millimeters.
    pub look_mm: [i32; 3],
    /// How well the two silhouettes separate on screen, `0..=1000`; 1000 means
    /// a clear gap, 0 means one fighter hides the other.
    pub readability_permille: u32,
    /// True when both fighters (silhouette included) sit inside the view cone.
    pub both_in_frame: bool,
}

fn to_i32(v: i64) -> i32 {
    i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX })
}

/// Frame a melee from the attacker's shoulder.
///
/// `half_fov_tan_permille` is the tangent of the horizontal half field of view
/// in permille (700 is about 35 degrees). The eye sits behind the attacker on
/// the attacker-to-defender line, pushed to one side by the least offset that
/// separates the two silhouettes (capped at [`SIDE_MAX_MM`]); close fights
/// that cap cannot separate report a lower readability instead of lying.
#[must_use]
pub fn frame_melee(
    attacker_mm: [i32; 2],
    defender_mm: [i32; 2],
    shoulder: Shoulder,
    half_fov_tan_permille: u32,
) -> Framing {
    let (ax, az) = (i64::from(attacker_mm[0]), i64::from(attacker_mm[1]));
    let (dx, dz) = (i64::from(defender_mm[0]), i64::from(defender_mm[1]));
    let (vx, vz) = (dx - ax, dz - az);
    let raw = (vx * vx + vz * vz).isqrt();
    let (ux, uz, dist) = if raw < MIN_DIST_MM {
        (0, 65_536, MIN_DIST_MM)
    } else {
        (vx * 65_536 / raw, vz * 65_536 / raw, raw)
    };
    let w = SILHOUETTE_HALF_MM;
    let back = (BACK_MIN_MM + dist / 3).min(BACK_MAX_MM);
    // Least side offset that clears the silhouettes: s >= w * (2b + d) / d.
    let needed = (w * (2 * back + dist) + dist - 1) / dist;
    let side = needed.clamp(SIDE_MIN_MM, SIDE_MAX_MM);
    let sign = match shoulder {
        Shoulder::Left => 1,
        Shoulder::Right => -1,
    };
    let (px, pz) = (-uz * sign, ux * sign);
    let ex = ax - ux * back / 65_536 + px * side / 65_536;
    let ez = az - uz * back / 65_536 + pz * side / 65_536;
    let lx = ax + vx * LOOK_BIAS_PERMILLE / 1000;
    let lz = az + vz * LOOK_BIAS_PERMILLE / 1000;

    let readability = (side * dist * 1000 / (w * (2 * back + dist))).min(1000);

    let (fx, fz) = (lx - ex, lz - ez);
    let flen = (fx * fx + fz * fz).isqrt().max(1);
    let limit = i64::from(half_fov_tan_permille);
    let inside = |x: i64, z: i64| {
        let (rx, rz) = (x - ex, z - ez);
        let depth = (fx * rx + fz * rz) / flen;
        let lateral = (fx * rz - fz * rx) / flen;
        depth > 0 && (lateral.abs() + w) * 1000 <= limit * depth
    };
    Framing {
        eye_mm: [to_i32(ex), EYE_HEIGHT_MM, to_i32(ez)],
        look_mm: [to_i32(lx), LOOK_HEIGHT_MM, to_i32(lz)],
        readability_permille: u32::try_from(readability).unwrap_or(0),
        both_in_frame: inside(ax, az) && inside(dx, dz),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOV_TAN: u32 = 700;
    /// Sixteen headings as integer unit-ish vectors (x, z) times 1000.
    const HEADINGS: [(i64, i64); 16] = [
        (1000, 0),
        (924, 383),
        (707, 707),
        (383, 924),
        (0, 1000),
        (-383, 924),
        (-707, 707),
        (-924, 383),
        (-1000, 0),
        (-924, -383),
        (-707, -707),
        (-383, -924),
        (0, -1000),
        (383, -924),
        (707, -707),
        (924, -383),
    ];

    fn pose(dist: i64, heading: (i64, i64), shoulder: Shoulder) -> Framing {
        let d = [
            to_i32(heading.0 * dist / 1000),
            to_i32(heading.1 * dist / 1000),
        ];
        frame_melee([0, 0], d, shoulder, FOV_TAN)
    }

    #[test]
    fn eighty_percent_of_melee_poses_read_and_all_hold_both_fighters() {
        let (mut total, mut readable) = (0_u32, 0_u32);
        for dist in (600..=6000).step_by(200) {
            for heading in HEADINGS {
                for shoulder in [Shoulder::Left, Shoulder::Right] {
                    let f = pose(dist, heading, shoulder);
                    total += 1;
                    readable += u32::from(f.readability_permille >= READABLE_PERMILLE);
                    assert!(f.both_in_frame, "dist {dist} heading {heading:?}");
                }
            }
        }
        assert!(readable * 100 >= total * 80, "{readable}/{total}");
    }

    #[test]
    fn camera_sits_behind_the_attacker_and_shoulders_mirror() {
        let (l, r) = (
            pose(2000, (1000, 0), Shoulder::Left),
            pose(2000, (1000, 0), Shoulder::Right),
        );
        assert!(l.eye_mm[0] < 0 && r.eye_mm[0] < 0, "behind the attacker");
        assert_eq!(l.eye_mm[0], r.eye_mm[0]);
        assert_eq!(l.eye_mm[2], -r.eye_mm[2]);
        assert_eq!(l.eye_mm[1], EYE_HEIGHT_MM);
        assert_eq!(l.look_mm[1], LOOK_HEIGHT_MM);
    }

    #[test]
    fn a_clinch_reports_poor_readability_not_a_lie() {
        let near = pose(600, (1000, 0), Shoulder::Left);
        let far = pose(3000, (1000, 0), Shoulder::Left);
        assert!(near.readability_permille < READABLE_PERMILLE);
        assert_eq!(far.readability_permille, 1000);
    }

    #[test]
    fn rotating_the_pair_rotates_the_pose() {
        // Quarter turn (x, z) -> (-z, x) keeps readability and frame state.
        let a = frame_melee([0, 0], [3000, 0], Shoulder::Left, FOV_TAN);
        let b = frame_melee([0, 0], [0, 3000], Shoulder::Left, FOV_TAN);
        assert_eq!(a.readability_permille, b.readability_permille);
        assert_eq!(a.both_in_frame, b.both_in_frame);
        assert_eq!((a.eye_mm[0], a.eye_mm[2]), (b.eye_mm[2], -b.eye_mm[0]));
    }

    #[test]
    fn coincident_fighters_do_not_panic_and_stay_finite() {
        let f = frame_melee([500, 500], [500, 500], Shoulder::Right, FOV_TAN);
        assert_eq!(
            f,
            frame_melee([500, 500], [500, 500], Shoulder::Right, FOV_TAN)
        );
        assert!(f.readability_permille <= 1000);
        let big = frame_melee(
            [1_000_000_000, 1_000_000_000],
            [-1_000_000_000, -1_000_000_000],
            Shoulder::Left,
            FOV_TAN,
        );
        assert_eq!(big.eye_mm[1], EYE_HEIGHT_MM);
    }
}
