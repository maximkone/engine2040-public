//! motion_warp: Unreal-style Motion Warping for melee root motion (AM40).
//!
//! Owner: `engine-anim`. One lunge or attack clip covers many distances: the
//! clip marks a warp window (a tick range of its [`RootTable`]); inside it the
//! clip's root motion is scaled and rotated (a skew warp) so the root ends at
//! a target, for example the blade reach point in front of the opponent,
//! instead of where the clip would put it. Outside the window the deltas are
//! the clip's own, bit for bit. No IK, no pose work: root motion only.
//!
//! Presentation only. The authority already moves the fighter on integers and
//! hashes it; this module never reads or writes the sim. The look uses the
//! warped table to draw the lunge so the clip's stride covers the displacement
//! the authority applies over the same ticks (see [`slide_mm`]). Units:
//! meters, radians, ticks at 60 Hz; the target and the deltas share one frame
//! (the caller picks it, usually the root's frame at the window start).
//!
//! Shape of the warp (what keeps it readable):
//! * skew: each in-window delta is scaled and turned by a weight that is 0 at
//!   both window edges and 1 on a plateau between them (smootherstep ramps
//!   over a quarter of the window each side), so the velocity at the edges is
//!   the clip's own and the lunge is not a lerp of the whole path;
//! * a few fixed passes re-aim the skew until the path nearly meets the target;
//!   scale and turn are bounded by [`WarpLimits`] (a lunge never stretches 5x
//!   or whips 150 degrees);
//! * the last miss (the part the bounded skew cannot cover, zero when the
//!   limits do not bind) is spread with smootherstep increments, so the root
//!   lands exactly on the target and the extra velocity is also 0 at the edges;
//! * yaw gets the same smootherstep spread, ending exactly on the target yaw.
//!
//! Determinism: f64 inside, `+ - * /` and `sqrt` only, no transcendentals, no
//! clock, no allocation beyond the output table; the same inputs give the same
//! bytes. Errors are [`WarpError`]; a rejected call returns no table.

use super::contract::{validate_root, RootDelta, RootTable};

/// Skew passes: each one re-aims the remaining miss through the same weights.
const SKEW_PASSES: usize = 4;
/// Share of the window each edge ramp takes before the plateau.
const RAMP: f64 = 0.25;
/// A clip path shorter than this (m) has no direction to turn or length to
/// scale; the whole miss goes through the smootherstep spread instead.
const MIN_PATH_M: f64 = 1.0e-4;
/// Shortest window (ticks): below this there is no edge to ease.
pub const MIN_WINDOW_TICKS: u32 = 4;
/// Largest absolute target coordinate or yaw accepted.
pub const MAX_ABS: f32 = 1.0e4;

/// A warp window on the clip's tick axis: deltas with
/// `start_tick <= tick < end_tick` are warped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WarpWindow {
    /// First warped tick.
    pub start_tick: u32,
    /// One past the last warped tick; the root at the window end is the root
    /// after the delta of tick `end_tick - 1`.
    pub end_tick: u32,
}

/// Where the root must be at the window end, relative to the window start.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WarpTarget {
    /// Total planar displacement `[dx, dz]` over the window, meters.
    pub dp: [f32; 2],
    /// Total yaw change over the window, radians; `None` keeps the clip's own.
    pub dyaw: Option<f32>,
}

/// Bounds on how far the skew may stretch or turn the clip.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WarpLimits {
    /// Smallest peak stride scale (a quarter lunge).
    pub scale_min: f32,
    /// Largest peak stride scale.
    pub scale_max: f32,
    /// Cosine of the widest peak turn of the path (0 = 90 degrees).
    pub min_turn_cos: f32,
}

impl Default for WarpLimits {
    fn default() -> Self {
        Self {
            scale_min: 0.4,
            scale_max: 2.5,
            min_turn_cos: 0.0,
        }
    }
}

/// What the warp did, for the receipt.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WarpReport {
    /// Peak stride scale the skew reached (1 = untouched).
    pub scale: f32,
    /// Cosine of the peak path turn the skew reached (1 = untouched).
    pub turn_cos: f32,
    /// Miss (m) the bounded skew left to the smootherstep spread.
    pub residual_m: f32,
    /// A limit held the skew back.
    pub clamped: bool,
}

/// Motion-warp failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WarpError {
    /// Non-finite or out-of-range value in the named lane.
    BadValue(&'static str),
    /// Window is shorter than [`MIN_WINDOW_TICKS`] or runs backwards; length carried.
    BadWindow(u32),
    /// The table does not hold every tick of the window; first missing tick carried.
    WindowGap(u32),
    /// The table itself is invalid (non-finite or out of order).
    BadTable,
    /// The limits are not `0 < min <= 1 <= max` or the cosine is outside `-0.9..1`.
    BadLimits,
}

/// Point `reach` meters short of `opponent` on the line from `from`, as a
/// displacement from `from`. Already inside reach: zero displacement (a lunge
/// never backs the fighter off). `None` on non-finite input or negative reach.
pub fn lunge_target(from: [f32; 2], opponent: [f32; 2], reach: f32) -> Option<[f32; 2]> {
    if !(from.iter().chain(opponent.iter()).all(|v| v.is_finite()) && reach.is_finite())
        || reach < 0.0
    {
        return None;
    }
    let (dx, dz) = (
        f64::from(opponent[0]) - f64::from(from[0]),
        f64::from(opponent[1]) - f64::from(from[1]),
    );
    let gap = (dx * dx + dz * dz).sqrt();
    let reach = f64::from(reach);
    if gap <= reach || gap < 1.0e-9 {
        return Some([0.0, 0.0]);
    }
    let k = (gap - reach) / gap;
    Some([(dx * k) as f32, (dz * k) as f32])
}

/// Quintic smootherstep: 0 at 0, 1 at 1, zero first and second derivative at both.
fn smoother(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

/// Skew weight of delta `i` of `n`: 0 at the edges, 1 on the plateau.
fn weight(i: usize, n: usize) -> f64 {
    let u = (i as f64 + 0.5) / n as f64;
    smoother(u.min(1.0 - u) / RAMP)
}

fn len(v: [f64; 2]) -> f64 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// Unit complex product `a * b` (a rotation composed with a rotation).
fn cmul(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] * b[0] - a[1] * b[1], a[0] * b[1] + a[1] * b[0]]
}

fn unit(v: [f64; 2]) -> [f64; 2] {
    let l = len(v);
    if l < 1.0e-12 {
        [1.0, 0.0]
    } else {
        [v[0] / l, v[1] / l]
    }
}

/// Rotation fraction `f` of the unit rotation `r`, by normalized lerp from identity.
fn rot_part(r: [f64; 2], f: f64) -> [f64; 2] {
    unit([(1.0 - f) + f * r[0], f * r[1]])
}

fn check_limits(l: &WarpLimits) -> Result<(), WarpError> {
    let ok = l.scale_min.is_finite()
        && l.scale_max.is_finite()
        && l.min_turn_cos.is_finite()
        && l.scale_min > 0.0
        && l.scale_min <= 1.0
        && l.scale_max >= 1.0
        && l.min_turn_cos > -0.9
        && l.min_turn_cos < 1.0;
    if ok {
        Ok(())
    } else {
        Err(WarpError::BadLimits)
    }
}

/// Warp the root motion of `table` inside `window` so the root ends at
/// `target`. Deltas outside the window are copied untouched; the window is
/// warped as described in the module doc. Returns the new table and a report.
pub fn warp_root(
    table: &RootTable,
    window: WarpWindow,
    target: WarpTarget,
    limits: WarpLimits,
) -> Result<(RootTable, WarpReport), WarpError> {
    check_limits(&limits)?;
    if target
        .dp
        .iter()
        .any(|v| !v.is_finite() || v.abs() > MAX_ABS)
    {
        return Err(WarpError::BadValue("target"));
    }
    if let Some(y) = target.dyaw {
        if !y.is_finite() || y.abs() > MAX_ABS {
            return Err(WarpError::BadValue("target yaw"));
        }
    }
    let width = window.end_tick.saturating_sub(window.start_tick);
    if window.end_tick <= window.start_tick || width < MIN_WINDOW_TICKS {
        return Err(WarpError::BadWindow(width));
    }
    validate_root(table).map_err(|_| WarpError::BadTable)?;
    // The window's deltas must be a contiguous run, one per tick.
    let first = table
        .deltas
        .iter()
        .position(|d| d.tick >= window.start_tick)
        .ok_or(WarpError::WindowGap(window.start_tick))?;
    let n = width as usize;
    for k in 0..n {
        let want = window.start_tick + k as u32;
        match table.deltas.get(first + k) {
            Some(d) if d.tick == want => {}
            _ => return Err(WarpError::WindowGap(want)),
        }
    }
    let span = &table.deltas[first..first + n];

    let raw: Vec<[f64; 2]> = span
        .iter()
        .map(|d| [f64::from(d.dp[0]), f64::from(d.dp[1])])
        .collect();
    let goal = [f64::from(target.dp[0]), f64::from(target.dp[1])];
    let sum = |v: &[[f64; 2]]| v.iter().fold([0.0, 0.0], |a, d| [a[0] + d[0], a[1] + d[1]]);
    let weights: Vec<f64> = (0..n).map(|i| weight(i, n)).collect();

    let mut d = raw.clone();
    let mut peak_scale = 1.0f64;
    let mut peak_rot = [1.0f64, 0.0];
    let mut clamped = false;
    let lg = len(goal);
    if len(sum(&raw)) > MIN_PATH_M {
        for _ in 0..SKEW_PASSES {
            let e = sum(&d);
            let le = len(e);
            if le < MIN_PATH_M {
                break;
            }
            // Full skew from the current end to the goal, then bound the peak.
            let want_s = lg / le;
            let lo = f64::from(limits.scale_min) / peak_scale;
            let hi = f64::from(limits.scale_max) / peak_scale;
            let s = want_s.clamp(lo, hi);
            clamped |= s != want_s;
            let mut r = if lg < MIN_PATH_M {
                [1.0, 0.0]
            } else {
                // goal * conj(end), normalized: the turn from the end to the goal.
                unit([
                    goal[0] * e[0] + goal[1] * e[1],
                    goal[1] * e[0] - goal[0] * e[1],
                ])
            };
            let mut cand = unit(cmul(peak_rot, r));
            let min_cos = f64::from(limits.min_turn_cos);
            if cand[0] < min_cos {
                clamped = true;
                let sin = (1.0 - min_cos * min_cos).sqrt();
                cand = [min_cos, if cand[1] < 0.0 { -sin } else { sin }];
                // New increment = cand * conj(peak_rot).
                r = unit(cmul(cand, [peak_rot[0], -peak_rot[1]]));
            }
            for (delta, w) in d.iter_mut().zip(&weights) {
                let scale = 1.0 + w * (s - 1.0);
                let rot = rot_part(r, *w);
                *delta = cmul(rot, [delta[0] * scale, delta[1] * scale]);
            }
            peak_scale *= s;
            peak_rot = cand;
        }
    }
    // Spread what the bounded skew left, smootherstep increments.
    let e = sum(&d);
    let miss = [goal[0] - e[0], goal[1] - e[1]];
    for (i, delta) in d.iter_mut().enumerate() {
        let inc = smoother((i as f64 + 1.0) / n as f64) - smoother(i as f64 / n as f64);
        delta[0] += miss[0] * inc;
        delta[1] += miss[1] * inc;
    }
    // Yaw: the same spread, so it also starts and ends with the clip's own rate.
    let yaw_raw: f64 = span.iter().map(|x| f64::from(x.dyaw)).sum();
    let yaw_miss = target.dyaw.map_or(0.0, |y| f64::from(y) - yaw_raw);
    let dyaw: Vec<f64> = span
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let inc = smoother((i as f64 + 1.0) / n as f64) - smoother(i as f64 / n as f64);
            f64::from(x.dyaw) + yaw_miss * inc
        })
        .collect();

    // Land exactly: the last delta is the remainder over the f32 values stored.
    let mut out = table.clone();
    let mut acc = [0.0f64; 2];
    let mut acc_yaw = 0.0f64;
    for (k, slot) in out.deltas[first..first + n].iter_mut().enumerate() {
        let last = k + 1 == n;
        let (dx, dz) = if last {
            (goal[0] - acc[0], goal[1] - acc[1])
        } else {
            (d[k][0], d[k][1])
        };
        let dy = if last {
            match target.dyaw {
                Some(y) => f64::from(y) - acc_yaw,
                None => dyaw[k],
            }
        } else {
            dyaw[k]
        };
        slot.dp = [dx as f32, dz as f32];
        slot.dyaw = dy as f32;
        acc[0] += f64::from(slot.dp[0]);
        acc[1] += f64::from(slot.dp[1]);
        acc_yaw += f64::from(slot.dyaw);
    }
    let report = WarpReport {
        scale: peak_scale as f32,
        turn_cos: peak_rot[0] as f32,
        residual_m: len(miss) as f32,
        clamped,
    };
    Ok((out, report))
}

/// Root position after every delta of `deltas` (planar, meters).
pub fn root_after(deltas: &[RootDelta]) -> [f32; 2] {
    let mut p = [0.0f64; 2];
    for d in deltas {
        p[0] += f64::from(d.dp[0]);
        p[1] += f64::from(d.dp[1]);
    }
    [p[0] as f32, p[1] as f32]
}

/// The foot slide a lunge draws: the widest gap in millimeters, over the
/// ticks both slices cover, between where the clip's root motion says the body
/// is and where the authority put it. A planted foot rides the authority
/// ground, so this is how far the clip's stride and the sand disagree. The
/// look wants it 0 at the end of the warp window and no larger than the
/// unwarped clip's anywhere inside it.
pub fn slide_mm(clip: &[RootDelta], authority: &[RootDelta]) -> f32 {
    let (mut c, mut a) = ([0.0f64; 2], [0.0f64; 2]);
    let mut worst = 0.0f64;
    for (x, y) in clip.iter().zip(authority) {
        for lane in 0..2 {
            c[lane] += f64::from(x.dp[lane]);
            a[lane] += f64::from(y.dp[lane]);
        }
        worst = worst.max(len([c[0] - a[0], c[1] - a[1]]));
    }
    (worst * 1000.0) as f32
}
