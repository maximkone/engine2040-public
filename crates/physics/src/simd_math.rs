//! simd_math: allocation-free lane-unrolled float slices for the tick.
//!
//! Safe 4-lane folds over `&[f32]` with no heap use: no `Vec`, no gather
//! buffer, members visited in index order, so every result is bit-equal to
//! the scalar loop it replaces. The lane width matches common SSE/Neon
//! vector widths and hands the autovectorizer straight-line chunks; there
//! are no `unsafe` blocks and no platform-gated intrinsics, so the bytes
//! agree on every PC (the rapier `simd` cargo feature stays off). Only
//! `+ - * /`, comparisons and integer math appear here, per the sim clippy
//! law. Caller: `crate::islands::update_sleep`, which folds per-island
//! motion peaks through [`island_peak`]. Gather-free by design: folding in
//! place over member indices beats a gather buffer [gems3 | 16.5 Tips &
//! Tricks].
#![forbid(unsafe_code)]

/// Lanes per unrolled chunk: one SSE/Neon-width group.
pub const LANES: usize = 4;

/// Fold one member into the running peak: in-range members compare their
/// `v2` entry against `peak` with `>`, exactly like the scalar loop.
#[inline]
fn fold_one(peak: &mut f32, v2: &[f32], i: usize, n: usize) {
    if i < n {
        if let Some(&v) = v2.get(i) {
            if v > *peak {
                *peak = v;
            }
        }
    }
}

/// Peak motion over an island's members: the maximum `v2[i]` for `i` in
/// `members` with `i < n`, floored at `0.0` (rest reads as zero, so an
/// empty or fully out-of-range island rests instead of erroring).
/// Out-of-range members and missing `v2` entries are ignored, never a
/// panic; NaN entries lose every `>` comparison, so they never become the
/// peak. Members fold in index order, four per chunk with a scalar tail:
/// the scalar `>` loop unrolled, bit-equal on every input, allocation-free
/// (slices in, one `f32` out).
#[must_use]
pub fn island_peak(v2: &[f32], members: &[usize], n: usize) -> f32 {
    debug_assert_eq!(LANES, 4, "unroll below names four lanes");
    let mut peak = 0.0f32;
    let mut chunks = members.chunks_exact(LANES);
    for chunk in &mut chunks {
        fold_one(&mut peak, v2, chunk[0], n);
        fold_one(&mut peak, v2, chunk[1], n);
        fold_one(&mut peak, v2, chunk[2], n);
        fold_one(&mut peak, v2, chunk[3], n);
    }
    for &i in chunks.remainder() {
        fold_one(&mut peak, v2, i, n);
    }
    peak
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar_peak(v2: &[f32], members: &[usize], n: usize) -> f32 {
        let mut peak = 0.0f32;
        for &i in members {
            if i < n {
                if let Some(&v) = v2.get(i) {
                    if v > peak {
                        peak = v;
                    }
                }
            }
        }
        peak
    }

    #[test]
    fn peak_covers_every_lane_boundary_bit_exact() {
        // Member counts 0..=9 cross every chunk/tail edge (4, 8).
        for len in 0..=9usize {
            let v2: Vec<f32> = (0..12)
                .map(|k| [-3.0, 0.0, 0.5, -0.0, 2.25, 7.0][k % 6])
                .collect();
            let members: Vec<usize> = (0..len).map(|k| (k * 5 + 1) % 14).collect();
            assert_eq!(
                island_peak(&v2, &members, v2.len()).to_bits(),
                scalar_peak(&v2, &members, v2.len()).to_bits(),
                "len {len}"
            );
        }
        // Monotone and single-spike shapes pin the running-maximum path.
        let rising: Vec<f32> = (0..16).map(|k| k as f32 * 0.25).collect();
        let all: Vec<usize> = (0..16).collect();
        assert_eq!(island_peak(&rising, &all, 16).to_bits(), 3.75f32.to_bits());
        let mut spike = vec![0.0f32; 16];
        spike[15] = 9.5;
        assert_eq!(island_peak(&spike, &all, 16).to_bits(), 9.5f32.to_bits());
        // All-negative motion floors at rest, like the scalar loop.
        let neg = vec![-1.0f32; 8];
        assert_eq!(island_peak(&neg, &all[..8], 8).to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn peak_ignores_hostile_inputs_without_panicking() {
        assert_eq!(island_peak(&[], &[], 0).to_bits(), 0.0f32.to_bits());
        assert_eq!(island_peak(&[], &[0, 1], 2).to_bits(), 0.0f32.to_bits());
        assert_eq!(
            island_peak(&[1.0], &[0, 1, 5, usize::MAX], 1).to_bits(),
            1.0f32.to_bits()
        );
        // NaN loses every comparison: the finite peak survives it.
        let nan_bits = island_peak(&[f32::NAN, 2.0], &[0, 1], 2).to_bits();
        assert_eq!(nan_bits, 2.0f32.to_bits());
        assert!(island_peak(&[f32::NAN], &[0], 1).is_finite());
        // Short `v2` and `n` smaller than the world both clamp safely.
        assert_eq!(
            island_peak(&[4.0, 1.0], &[0, 1, 2, 3], 4).to_bits(),
            4.0f32.to_bits()
        );
    }

    #[test]
    fn peak_repeats_bit_exact_x2() {
        let v2: Vec<f32> = (0..64).map(|k| (k as f32 * 1.7) % 5.0).collect();
        let members: Vec<usize> = (0..64).map(|k| (k * 37 + 11) % 70).collect();
        let first = island_peak(&v2, &members, v2.len());
        assert_eq!(
            island_peak(&v2, &members, v2.len()).to_bits(),
            first.to_bits()
        );
        assert_eq!(
            first.to_bits(),
            scalar_peak(&v2, &members, v2.len()).to_bits()
        );
    }

    #[test]
    fn peak_is_allocation_free() {
        use engine_core::alloc_probe;
        let v2: Vec<f32> = (0..128).map(|k| (k as f32 * 0.31) % 3.0).collect();
        let members: Vec<usize> = (0..128).map(|k| (k * 13 + 7) % 140).collect();
        let (peak, allocs) = alloc_probe::measure(|| {
            let mut acc = 0.0f32;
            for _ in 0..50 {
                acc += island_peak(&v2, &members, v2.len());
            }
            core::hint::black_box(acc)
        });
        assert!(peak.is_finite());
        assert_eq!(allocs.calls, 0, "peak fold allocates nothing");
    }
}
