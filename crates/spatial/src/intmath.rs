//! Integer helpers shared by the spatial verbs: floor square root and a
//! libm-free compass angle. No float reaches either, so answers are
//! bit-identical on every target.

use crate::sense_text::{cos_1k, sin_1k};

/// Floor of the square root of `n`; negative input reads as zero.
pub(crate) fn isqrt(n: i64) -> i64 {
    if n <= 0 {
        return 0;
    }
    // Newton from a power-of-two start above the root; integer only.
    let mut r = 1i64 << ((64 - n.leading_zeros()).div_ceil(2));
    loop {
        let next = (r + n / r) / 2;
        if next >= r {
            return r;
        }
        r = next;
    }
}

/// Whole degrees (0..=180, floored) between the facing `(dot > 0 is ahead)`
/// and a target at `(dot, lat)`: the largest `d` whose direction the target
/// has reached, by binary search over the 1024-scaled sine table.
pub(crate) fn angle_deg(dot: i64, lat: i64) -> u32 {
    let reached = |d: i32| i64::from(cos_1k(d)) * lat.abs() - i64::from(sin_1k(d)) * dot >= 0;
    let (mut lo, mut hi) = (0i32, 181i32);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if reached(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    u32::try_from(lo).unwrap_or(180)
}

/// Compass bearing in whole degrees, clockwise from the facing: 0 ahead, 90
/// right, 180 behind, 270 left. `lat > 0` is right. The magnitude off the
/// facing axis is floored on both sides; a target on the origin reads 0.
pub(crate) fn bearing_cw(dot: i64, lat: i64) -> u32 {
    if dot == 0 && lat == 0 {
        return 0;
    }
    let a = angle_deg(dot, lat);
    if lat < 0 {
        (360 - a) % 360
    } else {
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_is_the_floor_root() {
        for n in (0..5000i64).chain([1 << 40, (1 << 40) + 1, (1 << 40) - 1, i64::MAX / 4]) {
            let r = isqrt(n);
            assert!(r * r <= n && n < (r + 1) * (r + 1), "isqrt({n}) = {r}");
        }
        assert_eq!(isqrt(-4), 0);
    }

    #[test]
    fn bearing_hits_the_compass_points_and_halves() {
        assert_eq!(bearing_cw(100, 0), 0);
        assert_eq!(bearing_cw(0, 100), 90);
        assert_eq!(bearing_cw(-100, 0), 180);
        assert_eq!(bearing_cw(0, -100), 270);
        assert_eq!(bearing_cw(100, 100), 45);
        assert_eq!(bearing_cw(100, -100), 315);
        assert_eq!(bearing_cw(-100, 100), 135);
        assert_eq!(bearing_cw(0, 0), 0);
    }
}
