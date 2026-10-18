//! Cone questions for `scene.query`: integer tangent table, side and bearing
//! tests, the observer rules and the cone rows. Split out of the parent file
//! to keep it under the size ratchet; the parent re-exports every name.

use super::*;

/// Integer tangent table: `round(tan(d deg) * 1e6)` for `d` in 0..90. Cone and
/// bearing tests compare integers only, so the answer never touches libm.
pub(super) const TAN_E6: [i64; 90] = [
    0, 17455, 34921, 52408, 69927, 87489, 105104, 122785, 140541, 158384, 176327, 194380, 212557,
    230868, 249328, 267949, 286745, 305731, 324920, 344328, 363970, 383864, 404026, 424475, 445229,
    466308, 487733, 509525, 531709, 554309, 577350, 600861, 624869, 649408, 674509, 700208, 726543,
    753554, 781286, 809784, 839100, 869287, 900404, 932515, 965689, 1000000, 1035530, 1072369,
    1110613, 1150368, 1191754, 1234897, 1279942, 1327045, 1376382, 1428148, 1482561, 1539865,
    1600335, 1664279, 1732051, 1804048, 1880726, 1962611, 2050304, 2144507, 2246037, 2355852,
    2475087, 2605089, 2747477, 2904211, 3077684, 3270853, 3487414, 3732051, 4010781, 4331476,
    4704630, 5144554, 5671282, 6313752, 7115370, 8144346, 9514364, 11430052, 14300666, 19081137,
    28636253, 57289962,
];
/// Scale of `TAN_E6`: one unit of tangent is 1e6.
pub(super) const TAN_SCALE: i64 = 1_000_000;
/// Widest cone half-angle in whole degrees: the full circle.
pub(super) const CONE_MAX_HALF_DEG: usize = 180;
/// Front and rear arcs: within this many degrees of the facing reads ahead,
/// within it of the reverse reads behind, and the arcs between read left or
/// right by the sign of the lateral offset (right is positive).
pub(super) const CONE_SIDE_DEG: usize = 45;

/// Side of a cone row relative to the observer's facing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Within `CONE_SIDE_DEG` of the facing.
    Ahead,
    /// Left of the facing, outside the front and rear arcs.
    Left,
    /// Right of the facing, outside the front and rear arcs.
    Right,
    /// Within `CONE_SIDE_DEG` of the reverse facing.
    Behind,
}

impl Side {
    /// Door word for this side.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ahead => "ahead",
            Self::Left => "left",
            Self::Right => "right",
            Self::Behind => "behind",
        }
    }
}

/// One cone row: the actor id, plan-view distance in cm, side, and bearing in
/// whole degrees (floored, 0..=180) from the facing. Integers only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConeRow {
    pub id: u32,
    pub distance_cm: i64,
    pub side: Side,
    pub bearing_deg: u32,
}

/// Target inside `half` degrees of the facing, exact: on the front side
/// `|lat| / dot <= tan(half)`, on the rear side `>= tan(180 - half)`.
pub(super) fn within_cone(dot: i64, lat: i64, half: usize) -> bool {
    let off = lat.abs() * TAN_SCALE;
    if half >= CONE_MAX_HALF_DEG {
        true
    } else if half > 90 {
        dot >= 0 || off >= -dot * TAN_E6[180 - half]
    } else if half == 90 {
        dot >= 0
    } else {
        dot > 0 && off <= dot * TAN_E6[half]
    }
}

/// Angle from the facing is at least `d` degrees, for `d` in 0..=180.
pub(super) fn angle_at_least(dot: i64, lat: i64, d: usize) -> bool {
    let off = lat.abs() * TAN_SCALE;
    match d {
        0 => true,
        1..=89 => dot <= 0 || off >= dot * TAN_E6[d],
        90 => dot <= 0,
        91..=180 => dot < 0 && off <= -dot * TAN_E6[180 - d],
        _ => false,
    }
}

/// Floor of the bearing in whole degrees, 0..=180: the largest `d` the angle
/// reaches, by binary search over the monotone `angle_at_least`.
pub(super) fn bearing_deg(dot: i64, lat: i64) -> u32 {
    let (mut lo, mut hi) = (0usize, 181usize);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if angle_at_least(dot, lat, mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    u32::try_from(lo).unwrap_or(180)
}

/// Side from the integer tests: ahead in the front arc, behind in the rear
/// arc, else left or right by the lateral sign.
pub(super) fn side_of(dot: i64, lat: i64) -> Side {
    if within_cone(dot, lat, CONE_SIDE_DEG) {
        Side::Ahead
    } else if angle_at_least(dot, lat, 180 - CONE_SIDE_DEG) {
        Side::Behind
    } else if lat < 0 {
        Side::Left
    } else {
        Side::Right
    }
}

/// Recovery line for a refused cone: the exact call to repeat, observer named.
pub(super) fn cone_retry(observer: &str) -> String {
    format!(
        "retry scene.query {{\"kind\":\"cone\",{observer},\"radius_cm\":500,\"half_angle_deg\":45}}"
    )
}

/// Observer argument for an anchor retry.
pub(super) const CONE_ANCHOR_ARG: &str = "\"anchor\":\"<label from scene.brief>\"";
/// Observer argument for an eye-point retry.
pub(super) const CONE_EYE_ARG: &str = "\"eye_m\":[0.0,1.6,0.0],\"yaw\":0.0";

pub(super) fn check_cone_half(half: Option<i32>) -> Result<usize, QueryError> {
    let refuse =
        |message: String| QueryError::new("bad_limit", message, cone_retry(CONE_ANCHOR_ARG));
    let Some(h) = half else {
        return Err(refuse(format!(
            "cone needs half_angle_deg, the half-width in whole degrees 1..={CONE_MAX_HALF_DEG}"
        )));
    };
    match usize::try_from(h) {
        Ok(v) if (1..=CONE_MAX_HALF_DEG).contains(&v) => Ok(v),
        _ => Err(refuse(format!(
            "half_angle_deg {h} is outside 1..={CONE_MAX_HALF_DEG}"
        ))),
    }
}

/// Cone observer: plan point in cm, facing yaw, and the anchor id to leave out.
pub(super) type ConeObserver = ((i32, i32), f32, Option<u32>);

/// Observer for a cone: its plan point in cm, its facing yaw, and the anchor
/// id to leave out of the rows. An anchor faces its own yaw; an eye point
/// faces `yaw` (default 0). Exactly one of `anchor` and `eye_m`.
pub(super) fn cone_observer(
    actors: &[Actor],
    args: &QueryArgs,
) -> Result<ConeObserver, QueryError> {
    match (args.anchor, args.eye_m) {
        (Some(id), None) => {
            let refuse = |code: &'static str, message: String| {
                QueryError::new(code, message, cone_retry(CONE_ANCHOR_ARG))
            };
            let anchor = actor_by_id(actors, id).ok_or_else(|| {
                refuse("unknown_actor", format!("anchor {id} names no scene actor"))
            })?;
            let (cx, _, cz) = center_cm(anchor).ok_or_else(|| {
                refuse("unknown_actor", format!("anchor {id} has no finite center"))
            })?;
            Ok(((cx, cz), anchor.yaw, Some(id)))
        }
        (None, Some(_)) => {
            let refuse =
                |message: String| QueryError::new("bad_point", message, cone_retry(CONE_EYE_ARG));
            let eye = check_eye(args.eye_m).map_err(|e| refuse(e.message))?;
            let yaw = args.yaw.unwrap_or(0.0);
            if !yaw.is_finite() {
                return Err(refuse("yaw must be finite radians".to_owned()));
            }
            Ok(((to_cm(eye[0]), to_cm(eye[2])), yaw, None))
        }
        _ => Err(QueryError::new(
            "bad_point",
            "cone needs one observer: anchor, or eye_m with yaw, not both or neither",
            cone_retry(CONE_ANCHOR_ARG),
        )),
    }
}

/// Cone rows for one observer, in id order: every actor within `radius_cm`
/// (plan view) and within `half_angle_deg` of the facing. The facing snaps to
/// the solver's 45-degree octant. `filter` keeps actors whose name contains
/// it. Integer math only; an actor on the observer reads ahead at 0 cm.
pub fn cone_rows(actors: &[Actor], args: &QueryArgs) -> Result<Vec<ConeRow>, QueryError> {
    let half = check_cone_half(args.half_angle_deg)?;
    let radius = check_radius(args.radius_cm.unwrap_or(500))
        .map_err(|e| QueryError::new(e.code, e.message, cone_retry(CONE_ANCHOR_ARG)))?;
    let (origin, yaw, observer) = cone_observer(actors, args)?;
    let (fx, fz) = octant(yaw);
    let (fx, fz) = (i64::from(fx), i64::from(fz));
    let range2 = i128::from(radius) * i128::from(radius);
    let mut ordered: Vec<&Actor> = actors.iter().collect();
    ordered.sort_by_key(|a| a.id);
    let mut rows = Vec::new();
    for a in ordered {
        if observer == Some(a.id) {
            continue;
        }
        if args.filter.as_deref().is_some_and(|f| !a.name.contains(f)) {
            continue;
        }
        let Some(c) = center_cm(a) else { continue };
        let dx = i64::from(c.0) - i64::from(origin.0);
        let dz = i64::from(c.2) - i64::from(origin.1);
        let d2 = i128::from(dx) * i128::from(dx) + i128::from(dz) * i128::from(dz);
        if d2 > range2 {
            continue;
        }
        let dot = fx * dx + fz * dz;
        let lat = fx * dz - fz * dx;
        let (side, deg) = if dot == 0 && lat == 0 {
            (Side::Ahead, 0)
        } else if within_cone(dot, lat, half) {
            (side_of(dot, lat), bearing_deg(dot, lat))
        } else {
            continue;
        };
        rows.push(ConeRow {
            id: a.id,
            distance_cm: isqrt(i64::try_from(d2).unwrap_or(i64::MAX)),
            side,
            bearing_deg: deg,
        });
    }
    Ok(rows)
}
