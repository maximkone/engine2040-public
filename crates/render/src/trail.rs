//! Sword ribbon ring (RL53 slice 1): pure CPU trail staging.
//!
//! Owner: render lane. Units: mm for positions, mm for widths, 0..255 alpha.
//! Determinism: same pushes give same bytes twice; fixed order, no clock.
//! Errors: out-of-range index gives None, empty ring gives 0 segment.
//! Budget: fixed 16 samples, no heap per push; GPU draw lands next slice.

/// Fixed sample count for the ribbon ring.
pub const TRAIL_SAMPLES: usize = 16;

/// Q16 one (1.0) for [`catmull_q16`].
const Q16_ONE: i32 = 65_536;

/// Q16 half (0.5) for the one subdivision in [`max_seg_mm`].
const Q16_HALF: i32 = 32_768;

/// Thin end width in mm for [`width_mm`].
const WIDTH_THIN_MM: u32 = 2;

/// Extra width at the middle in mm for [`width_mm`].
const WIDTH_PEAK_EXTRA_MM: u32 = 22;

/// Squared half-range for the parabolic width falloff.
const WIDTH_RANGE_SQ: i32 = 225;

/// Divisor matching [`WIDTH_RANGE_SQ`] minus the centre square.
const WIDTH_DIVISOR: u32 = 224;

/// Fixed ring of sword ribbon samples in mm.
///
/// Holds the last [`TRAIL_SAMPLES`] base/tip pairs in push order. Push
/// overwrites the oldest sample once full, so `len` caps at 16 and
/// `get(0)` is always the oldest resident sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrailRing {
    base: [[i32; 3]; TRAIL_SAMPLES],
    tip: [[i32; 3]; TRAIL_SAMPLES],
    len: usize,
    head: usize,
}

impl Default for TrailRing {
    fn default() -> Self {
        Self::new()
    }
}

impl TrailRing {
    /// Empty ring holding no samples.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: [[0; 3]; TRAIL_SAMPLES],
            tip: [[0; 3]; TRAIL_SAMPLES],
            len: 0,
            head: 0,
        }
    }

    /// Drop all samples; backing stays zeroed for the next slash.
    pub fn clear(&mut self) {
        self.len = 0;
        self.head = 0;
    }

    /// Push one base/tip pair in mm; overwrites oldest once full.
    ///
    /// Steady path: two array stores plus counter updates, no heap.
    pub fn push(&mut self, base_mm: [i32; 3], tip_mm: [i32; 3]) {
        self.base[self.head] = base_mm;
        self.tip[self.head] = tip_mm;
        self.head = (self.head + 1) % TRAIL_SAMPLES;
        if self.len < TRAIL_SAMPLES {
            self.len += 1;
        }
    }

    /// Resident sample count, capped at [`TRAIL_SAMPLES`].
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when no samples are resident.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Oldest-first sample; index 0 is the tail, `len - 1` is the head.
    ///
    /// Returns `None` past `len` so a stale index never panics.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<([i32; 3], [i32; 3])> {
        if index >= self.len {
            return None;
        }
        let slot = (self.head + TRAIL_SAMPLES - self.len + index) % TRAIL_SAMPLES;
        Some((self.base[slot], self.tip[slot]))
    }
}

/// Ribbon width in mm at oldest-first index `i`.
///
/// Parabolic falloff: thin at both ends, wide at the middle. Index past
/// 15 clamps to the head end so a stale index still draws thin.
#[must_use]
pub fn width_mm(i: usize) -> u32 {
    let clamped = i.min(TRAIL_SAMPLES - 1);
    let as_i32 = i32::try_from(clamped).unwrap_or(0);
    let dist = (2 * as_i32 - 15).abs();
    let numer = WIDTH_RANGE_SQ - dist * dist;
    let numer_u32 = u32::try_from(numer.max(0)).unwrap_or(0);
    WIDTH_THIN_MM + (WIDTH_PEAK_EXTRA_MM * numer_u32) / WIDTH_DIVISOR
}

/// Ribbon alpha at oldest-first index `i`.
///
/// Linear ramp: 0 at the tail, 255 at the head. Index past 15 clamps
/// to opaque so a stale index never turns transparent.
#[must_use]
pub fn alpha_u8(i: usize) -> u8 {
    let clamped = i.min(TRAIL_SAMPLES - 1);
    let num = u32::try_from(clamped).unwrap_or(u32::MAX);
    let den = u32::try_from(TRAIL_SAMPLES - 1).unwrap_or(1);
    u8::try_from((255 * num) / den).unwrap_or(u8::MAX)
}

/// One-dimension Catmull-Rom in Q16, integer only.
///
/// `t_q16` is Q16 in 0..=65536 (0.0..=1.0); out-of-range clamps to the
/// endpoints. Endpoints are exact: `t = 0` gives `p1`, `t = 65536`
/// gives `p2`. Intermediate math runs in `i64` so mm inputs never
/// overflow; the halved sum clamps to `i32` range.
#[must_use]
pub fn catmull_q16(p0: i32, p1: i32, p2: i32, p3: i32, t_q16: i32) -> i32 {
    let t = i64::from(t_q16.clamp(0, Q16_ONE));
    let t2 = (t * t) >> 16;
    let t3 = (t2 * t) >> 16;
    let q0 = i64::from(p0);
    let q1 = i64::from(p1);
    let q2 = i64::from(p2);
    let q3 = i64::from(p3);
    let term1 = 2 * q1;
    let term2 = ((-q0 + q2) * t) >> 16;
    let c2 = 2 * q0 - 5 * q1 + 4 * q2 - q3;
    let term3 = (c2 * t2) >> 16;
    let c3 = -q0 + 3 * q1 - 3 * q2 + q3;
    let term4 = (c3 * t3) >> 16;
    let half = (term1 + term2 + term3 + term4) >> 1;
    let clamped = half.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    i32::try_from(clamped).unwrap_or(0)
}

/// Integer distance in mm between two mm points.
fn dist_mm(a: [i32; 3], b: [i32; 3]) -> u32 {
    let dx = i64::from(a[0]) - i64::from(b[0]);
    let dy = i64::from(a[1]) - i64::from(b[1]);
    let dz = i64::from(a[2]) - i64::from(b[2]);
    let sum = dx * dx + dy * dy + dz * dz;
    let mag = u64::try_from(sum.max(0)).unwrap_or(0).isqrt();
    u32::try_from(mag).unwrap_or(u32::MAX)
}

/// Smoothed midpoint of one span per axis at `t = 0.5`.
fn span_mid(prev: [i32; 3], a: [i32; 3], b: [i32; 3], next: [i32; 3]) -> [i32; 3] {
    [
        catmull_q16(prev[0], a[0], b[0], next[0], Q16_HALF),
        catmull_q16(prev[1], a[1], b[1], next[1], Q16_HALF),
        catmull_q16(prev[2], a[2], b[2], next[2], Q16_HALF),
    ]
}

/// Longest smoothed sub-segment in mm over one midpoint per span.
///
/// Each span `i -> i + 1` is split at its Catmull-Rom midpoint (ends
/// clamp by duplicating the endpoint), for both the base and the tip
/// lanes; the longest half-segment wins. Empty and single-sample rings
/// give 0. Integer only; same ring gives the same length twice.
#[must_use]
pub fn max_seg_mm(ring: &TrailRing) -> u32 {
    if ring.len() < 2 {
        return 0;
    }
    let mut worst: u32 = 0;
    for s in 0..ring.len() - 1 {
        let (prev_base, prev_tip) = ring.get(s.saturating_sub(1)).unwrap_or(([0; 3], [0; 3]));
        let (a_base, a_tip) = ring.get(s).unwrap_or(([0; 3], [0; 3]));
        let (b_base, b_tip) = ring.get(s + 1).unwrap_or(([0; 3], [0; 3]));
        let next_slot = if s + 2 < ring.len() { s + 2 } else { s + 1 };
        let (next_base, next_tip) = ring.get(next_slot).unwrap_or(([0; 3], [0; 3]));
        // Clamp the Catmull ends: duplicate the span end when past the ring.
        let (p0_base, p0_tip) = if s == 0 {
            (a_base, a_tip)
        } else {
            (prev_base, prev_tip)
        };
        let mid_base = span_mid(p0_base, a_base, b_base, next_base);
        let mid_tip = span_mid(p0_tip, a_tip, b_tip, next_tip);
        for d in [
            dist_mm(a_base, mid_base),
            dist_mm(mid_base, b_base),
            dist_mm(a_tip, mid_tip),
            dist_mm(mid_tip, b_tip),
        ] {
            if d > worst {
                worst = d;
            }
        }
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    /// Staged slash tip at sample `i`: -500..500 mm along x.
    fn slash_tip(i: usize) -> [i32; 3] {
        let step = i32::try_from(i).unwrap_or(0);
        [-500 + (1_000 * step) / 15, 0, 0]
    }

    /// Staged slash base: same x, 80 mm below the tip on y.
    fn slash_base(i: usize) -> [i32; 3] {
        let tip = slash_tip(i);
        [tip[0], -80, 0]
    }

    /// Fill a ring with the 16 staged slash samples.
    fn staged_ring() -> TrailRing {
        let mut ring = TrailRing::new();
        for i in 0..TRAIL_SAMPLES {
            ring.push(slash_base(i), slash_tip(i));
        }
        ring
    }

    #[test]
    fn trail_ring_holds_16_and_replays_bytes_x2() {
        let first = staged_ring();
        let mut second = TrailRing::new();
        for i in 0..TRAIL_SAMPLES {
            second.push(slash_base(i), slash_tip(i));
        }
        assert_eq!(first.len(), TRAIL_SAMPLES);
        assert_eq!(second.len(), TRAIL_SAMPLES);
        for i in 0..TRAIL_SAMPLES {
            assert_eq!(first.get(i), second.get(i));
        }
        assert_eq!(first, second);
        // Ring overwrites oldest once full: four more pushes drop 0..4.
        let mut grown = first.clone();
        for k in 0..4 {
            grown.push([2_000 + k32(k), 0, 0], [2_000 + k32(k), 60, 0]);
        }
        assert_eq!(grown.len(), TRAIL_SAMPLES);
        let (old_base, _) = grown.get(0).expect("ring keeps 16");
        assert_eq!(old_base[0], slash_base(4)[0]);
    }

    /// Small helper keeps the overwrite probe in `i32` without a cast.
    fn k32(k: usize) -> i32 {
        i32::try_from(k).unwrap_or(0)
    }

    #[test]
    fn trail_tip_matches_sweep_p1_within_10mm() {
        let ring = staged_ring();
        let (_, tip) = ring.get(TRAIL_SAMPLES - 1).expect("head resident");
        let want: [i32; 3] = [500, 0, 0];
        assert_eq!(dist_mm(tip, want), 0);
        assert!(
            dist_mm(tip, want) <= 10,
            "tip {tip:?} drifts past 10 mm from {want:?}"
        );
    }

    #[test]
    fn trail_smoothed_max_segment_within_80mm() {
        let ring = staged_ring();
        let worst = max_seg_mm(&ring);
        assert!(
            worst <= 80,
            "smoothed max segment {worst} mm runs past the 80 mm kill line"
        );
        assert!(worst > 0, "staged slash must span nonzero segments");
    }

    #[test]
    fn trail_width_is_thin_at_ends_and_wide_in_middle() {
        assert_eq!(width_mm(0), WIDTH_THIN_MM);
        assert_eq!(width_mm(TRAIL_SAMPLES - 1), WIDTH_THIN_MM);
        let mid = width_mm(7);
        assert_eq!(mid, width_mm(8));
        assert!(mid > width_mm(0));
        assert_eq!(mid, WIDTH_THIN_MM + WIDTH_PEAK_EXTRA_MM);
        assert_eq!(width_mm(99), width_mm(TRAIL_SAMPLES - 1));
    }

    #[test]
    fn trail_alpha_ramps_from_tail_to_head() {
        assert_eq!(alpha_u8(0), 0);
        assert_eq!(alpha_u8(TRAIL_SAMPLES - 1), 255);
        assert!(alpha_u8(7) < alpha_u8(8));
        assert_eq!(alpha_u8(99), 255);
    }

    #[test]
    fn trail_catmull_hits_endpoints_and_midpoint() {
        assert_eq!(catmull_q16(0, 10, 20, 30, 0), 10);
        assert_eq!(catmull_q16(0, 10, 20, 30, Q16_ONE), 20);
        assert_eq!(catmull_q16(-500, -500, 500, 500, Q16_HALF), 0);
        assert_eq!(catmull_q16(5, 5, 5, 5, Q16_HALF), 5);
    }

    #[test]
    fn trail_push_is_zero_alloc_and_clears() {
        let mut ring = TrailRing::new();
        let ((), count) = measure(|| {
            for i in 0..TRAIL_SAMPLES {
                ring.push(slash_base(i), slash_tip(i));
            }
        });
        assert_eq!(count.calls, 0, "push must not touch the heap");
        assert_eq!(ring.len(), TRAIL_SAMPLES);
        ring.clear();
        assert_eq!(ring.len(), 0);
        assert!(ring.is_empty());
        assert_eq!(ring.get(0), None);
        assert_eq!(max_seg_mm(&ring), 0);
    }
}
