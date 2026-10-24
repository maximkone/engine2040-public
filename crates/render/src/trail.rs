//! Sword ribbon ring (RL53 slice 1): the CPU ring the sim's integer weapon
//! sweeps feed, plus the vert strip the staged ribbon draw reads.
//!
//! Owner: render lane. Units: mm in every stored sample and width, m in vert
//! positions, alpha share 0..=1. Determinism: the samples are exact integer
//! copies, so the same pushes give the same ring bytes and the same verts twice;
//! no clock, no float in a stored sample. Errors: an index past `len` is `None`,
//! an empty ring is 0 verts. Budget: 16 samples, one bounded vert `Vec` per
//! build. Consumer: the `crates/game` trail feed pushes the swing's sweep
//! segments; the staged `shaders/trail.wgsl` expands the strip next slice.

use crate::draw::TrailStripVert;

/// Ring capacity: the last 16 tick segments of one swing.
pub const TRAIL_SAMPLES: usize = 16;
/// Longest kept smoothed sub-segment in cm: the row's smoothing cap.
pub const TRAIL_BLADE_MAX_CM: u32 = 8;
/// [`TRAIL_BLADE_MAX_CM`] in mm, the drop line the smoothing check reads.
const TRAIL_MAX_MM: u32 = TRAIL_BLADE_MAX_CM * 10;
/// Thin width in mm when the width curve has no entry for a sample.
const TRAIL_THIN_MM: u32 = 2;
/// Modeled nanoseconds charged per ribbon vert by [`TrailRing::trail_ms`].
const TRAIL_NS_PER_VERT: f64 = 4.0;

/// One sweep segment kept by the ring: exact integer millimeters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrailSample {
    /// Blade start of the tick in mm (`p0_mm`).
    pub p0_mm: [i32; 3],
    /// Blade end of the tick in mm (`p1_mm`).
    pub p1_mm: [i32; 3],
}

/// Fixed oldest-first ring of the last [`TRAIL_SAMPLES`] segments: `newest` is
/// the newest slot, `len` the resident count, and a push past full overwrites
/// the oldest resident sample.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrailRing {
    samples: [TrailSample; TRAIL_SAMPLES],
    newest: usize,
    len: usize,
}

impl TrailRing {
    /// Resident sample count, capped at [`TRAIL_SAMPLES`].
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when no sample is resident.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Push one tick's sweep segment in mm; both endpoints are copied as
    /// integers, never converted to float.
    pub fn push_segment(&mut self, sweep_p0_mm: [i32; 3], sweep_p1_mm: [i32; 3]) {
        self.newest = (self.newest + 1) % TRAIL_SAMPLES;
        self.samples[self.newest] = TrailSample {
            p0_mm: sweep_p0_mm,
            p1_mm: sweep_p1_mm,
        };
        if self.len < TRAIL_SAMPLES {
            self.len += 1;
        }
    }

    /// Oldest-first sample; `None` past `len`, so a stale index never panics.
    #[must_use]
    pub(crate) fn sample(&self, index: usize) -> Option<TrailSample> {
        if index >= self.len {
            return None;
        }
        let slot = (self.newest + TRAIL_SAMPLES + 1 - self.len + index) % TRAIL_SAMPLES;
        Some(self.samples[slot])
    }

    /// Newest pushed `p1_mm`: the sweep's own blade end, so the ribbon tip
    /// stays inside the sim's centimeter.
    #[must_use]
    pub fn tip_mm(&self) -> Option<[i32; 3]> {
        let newest = self.len().checked_sub(1)?;
        self.sample(newest).map(|s| s.p1_mm)
    }

    /// Build the ribbon vert strip from the resident samples.
    ///
    /// One base vert and one tip vert per output point, oldest first, in the
    /// `draw::trail_strip_verts` strip shape. Each span is split at its integer
    /// Catmull-Rom midpoint, so the ribbon bends between ticks; the width curve
    /// lays the strip across the posed segment (`width_mm` holds one width per
    /// oldest-first sample, a missing entry reuses the last one); alpha ramps 0
    /// at the oldest sample to 1 at the newest. A span whose smoothed halves run
    /// past [`TRAIL_BLADE_MAX_CM`] on either lane drops that sample; one bounded
    /// `Vec` (at most 64 verts), and the same ring gives the same verts twice.
    #[must_use]
    pub fn ribbon_verts(&self, width_mm: &[u32]) -> Vec<TrailStripVert> {
        let len = self.len();
        let mut out = Vec::with_capacity(4 * TRAIL_SAMPLES);
        // Lane pair of the last kept sample, of the one before it, plus the
        // index of the last kept sample; empty until the first accept.
        let mut prev = [[0i32; 3]; 2];
        let mut prev2 = [[0i32; 3]; 2];
        let mut kept = 0usize;
        let mut last = 0usize;
        for i in 0..len {
            let Some(s) = self.sample(i) else { continue };
            let here = [s.p0_mm, laid_tip(s.p0_mm, s.p1_mm, width_at(width_mm, i))];
            if kept == 0 {
                out.push(vert(here[0], age_alpha(i, len)));
                out.push(vert(here[1], age_alpha(i, len)));
                prev = here;
                prev2 = here;
                last = i;
                kept = 1;
                continue;
            }
            let next = self.sample(i + 1).unwrap_or(s);
            let after = [
                next.p0_mm,
                laid_tip(next.p0_mm, next.p1_mm, width_at(width_mm, i + 1)),
            ];
            let mut mid = [[0i32; 3]; 2];
            let mut wild = false;
            for l in 0..2 {
                mid[l] = cr3(prev2[l], prev[l], here[l], after[l]);
                wild |=
                    dist(prev[l], mid[l]) > TRAIL_MAX_MM || dist(mid[l], here[l]) > TRAIL_MAX_MM;
            }
            if wild {
                continue;
            }
            let alpha = age_alpha(last, len);
            out.push(vert(mid[0], alpha));
            out.push(vert(mid[1], alpha));
            let here_alpha = age_alpha(i, len);
            out.push(vert(here[0], here_alpha));
            out.push(vert(here[1], here_alpha));
            prev2 = prev;
            prev = here;
            last = i;
            kept += 1;
        }
        out
    }

    /// Modeled build cost in ms for `frames` ribbon builds, one per frame.
    ///
    /// Pure model, no wall clock: a build is charged four verts per resident
    /// sample at `TRAIL_NS_PER_VERT`, so every host reads the same number.
    #[must_use]
    pub fn trail_ms(&self, frames: u32) -> f64 {
        let verts = u32::try_from(4 * self.len).unwrap_or(u32::MAX);
        f64::from(verts) * f64::from(frames) * TRAIL_NS_PER_VERT / 1_000_000.0
    }
}

/// Width in mm at oldest-first sample `i`; a missing entry reuses the last one.
fn width_at(width_mm: &[u32], i: usize) -> u32 {
    width_mm
        .get(i)
        .or_else(|| width_mm.last())
        .copied()
        .unwrap_or(TRAIL_THIN_MM)
}

/// Alpha share for the sample pushed `i` ticks ago: 0 at the oldest, 1 newest.
fn age_alpha(i: usize, len: usize) -> f32 {
    let den = len.saturating_sub(1);
    if den == 0 {
        return 1.0;
    }
    let num = u16::try_from(i.min(den)).unwrap_or(u16::MAX);
    f32::from(num) / f32::from(u16::try_from(den).unwrap_or(u16::MAX))
}

/// Lay the width curve across the posed segment: the tip point sits `width_mm`
/// from the base along `p1 - p0`, in integers; a zero-length segment keeps the
/// posed tip, so there is no divide by zero.
fn laid_tip(p0_mm: [i32; 3], p1_mm: [i32; 3], width_mm: u32) -> [i32; 3] {
    let len = dist(p0_mm, p1_mm);
    if len == 0 {
        return p1_mm;
    }
    let w = i64::from(width_mm);
    let mut out = p0_mm;
    for k in 0..3 {
        let step = i64::from(p1_mm[k]) - i64::from(p0_mm[k]);
        out[k] = p0_mm[k].saturating_add(i32::try_from(step * w / i64::from(len)).unwrap_or(0));
    }
    out
}

/// Catmull-Rom midpoint (t = 1/2) of the span `a -> b` in integers,
/// `(-p0 + 9a + 9b - p3) / 16`, so every host reads the same midpoint.
fn cr(p0: i32, p1: i32, p2: i32, p3: i32) -> i32 {
    let sum = 9 * (i64::from(p1) + i64::from(p2)) - i64::from(p0) - i64::from(p3);
    i32::try_from(sum / 16).unwrap_or(if sum < 0 { i32::MIN } else { i32::MAX })
}

/// Per-axis [`cr`] over one span.
fn cr3(p0: [i32; 3], a: [i32; 3], b: [i32; 3], p3: [i32; 3]) -> [i32; 3] {
    [
        cr(p0[0], a[0], b[0], p3[0]),
        cr(p0[1], a[1], b[1], p3[1]),
        cr(p0[2], a[2], b[2], p3[2]),
    ]
}

/// Integer distance in mm between two mm points: i64 inside, `isqrt` out.
fn dist(a: [i32; 3], b: [i32; 3]) -> u32 {
    let mut sum = 0u64;
    for k in 0..3 {
        let d = i64::from(a[k]) - i64::from(b[k]);
        sum += u64::try_from(d * d).unwrap_or(0);
    }
    u32::try_from(sum.isqrt()).unwrap_or(u32::MAX)
}

/// One vert from an mm point and a 0..=1 age alpha.
fn vert(p_mm: [i32; 3], alpha: f32) -> TrailStripVert {
    TrailStripVert {
        pos_m: [m_mm(p_mm[0]), m_mm(p_mm[1]), m_mm(p_mm[2])],
        alpha,
    }
}

/// Integer mm to meters without an `as` cast: whole meters plus the remainder.
fn m_mm(mm: i32) -> f32 {
    let (whole, rem) = (mm / 1000, mm % 1000);
    let whole16 = i16::try_from(whole).unwrap_or(if whole < 0 { i16::MIN } else { i16::MAX });
    f32::from(whole16) + f32::from(i16::try_from(rem).unwrap_or(0)) / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    /// Staged slash: the blade start walks -500..=500 mm in x, tip 80 mm above.
    fn sweep(i: usize) -> ([i32; 3], [i32; 3]) {
        let p1 = [-500 + (1_000 * i32::try_from(i).unwrap_or(0)) / 15, 0, 0];
        ([p1[0], -80, 0], p1)
    }

    /// The row's width curve: 2 mm at the thin end, 24 mm at the middle.
    fn curve() -> [u32; TRAIL_SAMPLES] {
        core::array::from_fn(|i| TRAIL_THIN_MM + 22 * u32::try_from(i).unwrap_or(0) / 15)
    }

    /// Ring filled with the 16 staged slash segments.
    fn staged() -> TrailRing {
        let mut ring = TrailRing::default();
        for i in 0..TRAIL_SAMPLES {
            let (p0, p1) = sweep(i);
            ring.push_segment(p0, p1);
        }
        ring
    }

    /// Span of one lane segment in meters, for the 8 cm check.
    fn span_m(a: &TrailStripVert, b: &TrailStripVert) -> f32 {
        let d = [
            b.pos_m[0] - a.pos_m[0],
            b.pos_m[1] - a.pos_m[1],
            b.pos_m[2] - a.pos_m[2],
        ];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }

    #[test]
    fn trail_ring_bytes_x2_and_tip_matches_sweep_p1() {
        let a = staged();
        let b = staged();
        let (va, vb) = (a.ribbon_verts(&curve()), b.ribbon_verts(&curve()));
        assert_eq!(a, b, "the same pushes write the same ring bytes");
        assert_eq!(va, vb, "the same ring gives the same verts twice");
        assert!(va.len() > 2 * TRAIL_SAMPLES, "the spans subdivide");
        let tip = a.tip_mm().expect("the ring holds the staged slash");
        let (_, want) = sweep(TRAIL_SAMPLES - 1);
        let drift = dist(tip, want);
        assert!(drift <= 10, "tip {tip:?} drifts {drift} mm from sweep p1");
    }

    #[test]
    fn trail_ring_caps_at_sixteen_and_keeps_order() {
        let mut ring = TrailRing::default();
        for i in 0..20 {
            let (p0, p1) = sweep(i);
            ring.push_segment(p0, p1);
        }
        assert_eq!(ring.len(), TRAIL_SAMPLES, "20 pushes keep 16 samples");
        let kept: Vec<i32> = (0..ring.len())
            .map(|i| ring.sample(i).expect("resident").p0_mm[0])
            .collect();
        let want: Vec<i32> = (4..20).map(|i| sweep(i).0[0]).collect();
        assert_eq!(kept, want, "the newest 16 survive, oldest first");
        let ordered = kept.windows(2).all(|w| w[0] < w[1]);
        assert!(ordered, "order is oldest-first");
        let ((), probe) = measure(|| {
            let (p0, p1) = sweep(0);
            ring.push_segment(p0, p1);
        });
        assert_eq!(probe.calls, 0, "a push never touches the heap");
    }

    #[test]
    fn trail_smoothing_never_leaves_a_segment_over_eight_cm() {
        let mut ring = TrailRing::default();
        for i in 0..TRAIL_SAMPLES {
            let (p0, p1) = sweep(i);
            if i == 8 {
                // One wild tick: the ring must drop it, not stretch the ribbon.
                ring.push_segment([8_000, 0, 0], [8_000, 500, 0]);
            } else {
                ring.push_segment(p0, p1);
            }
        }
        let verts = ring.ribbon_verts(&curve());
        let mut longest = 0.0f32;
        for lane in 0..2 {
            for step in 0..verts.len() / 2 - 1 {
                let (a, b) = (2 * step + lane, 2 * step + 2 + lane);
                longest = longest.max(span_m(&verts[a], &verts[b]));
            }
        }
        assert!(longest > 0.0, "the staged slash still spans samples");
        assert!(longest <= 0.080, "kept segment {longest} m is past 8 cm");
        let wild = verts.iter().any(|v| (v.pos_m[0] - 8.0).abs() < 0.05);
        assert!(!wild, "the wild sample is dropped, not kept");
    }

    #[test]
    fn trail_ms_budget() {
        let ms = staged().trail_ms(300);
        assert!(ms <= 0.15, "300 frames of a 16-sample ring cost {ms} ms");
        assert!(ms > 0.0, "a full ring costs something");
        assert_eq!(TrailRing::default().trail_ms(300), 0.0, "empty builds 0");
    }
}
