//! edit: clip edit verbs trim, retime, cleanup over [`QClip`]/[`QTrack`].
//!
//! Owner: `engine-anim`; consumer: the crate root wires the module
//! (`pub mod edit` in `lib.rs`; a follow-up leg extends this file with
//! strip and mirror). Data owner `clip.rs` lends its quantized types
//! read-only; this module never edits them in place. Units: ticks at
//! 60 Hz for windows, seconds for key times, meters for values.
//! Determinism: `sample_track` reads plus `+ - * /`, `round` and `sqrt`
//! only, stable track order; same clip and args give same bytes x2.
//! Errors: `None` for empty, reversed or out-of-range windows and ratios;
//! `cleanup` never fails (bad tolerance reads 0). Budget: each verb
//! allocates one new clip (authoring-time only, never the tick).

use engine_core::vec3::dist2;

use super::clip::{bake_clip, bake_track, sample_track, QClip, QTrack, TICK_DT};

/// Integer tick count to float without a numeric cast.
///
/// Owner: `engine-anim`; consumer: `trim` plus `retime` tick-to-second math.
/// Units: counts to float; determinism: integer bit ops only, same input
/// gives same bits twice (matches correctly-rounded conversion, pinned below).
/// Errors: never fails; zero maps to zero, max rounds to two to the 32.
fn u32_to_f32(x: u32) -> f32 {
    if x == 0 {
        return 0.0;
    }
    let e = 32 - x.leading_zeros();
    let mut exp = 127 + e - 1;
    if e <= 24 {
        let hidden = 1u32 << (e - 1);
        let mant = (x ^ hidden) << (23 - (e - 1));
        return f32::from_bits((exp << 23) | mant);
    }
    let shift = e - 24;
    let kept = x >> shift;
    let dropped = x & ((1u32 << shift) - 1);
    let half = 1u32 << (shift - 1);
    let mut mant = kept & 0x7FFFFF;
    let round_up = dropped > half || (dropped == half && (mant & 1) == 1);
    if round_up {
        if mant == 0x7FFFFF {
            mant = 0;
            exp += 1;
        } else {
            mant += 1;
        }
    }
    f32::from_bits((exp << 23) | mant)
}

/// Value of `tr` at `t` through the shared sampler.
fn at(tr: &QTrack, t: f32) -> [f32; 3] {
    let mut hint = 0usize;
    sample_track(tr, t, &mut hint)
}

/// Linear prediction of `v` between `a` at `ta` and `b` at `tb`, evaluated at `t`.
fn lerp3(a: [f32; 3], b: [f32; 3], ta: f32, tb: f32, t: f32) -> [f32; 3] {
    let f = if tb > ta {
        ((t - ta) / (tb - ta)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    [
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
    ]
}

/// Window `[start_tick, end_tick]` as a new clip with times shifted to 0.
///
/// Keeps keys inside the window plus boundary keys sampled at both ends;
/// `None` when the window is empty or reversed.
pub fn trim(clip: &QClip, start_tick: u32, end_tick: u32) -> Option<QClip> {
    if end_tick <= start_tick {
        return None;
    }
    let (t0, t1) = (
        u32_to_f32(start_tick) * TICK_DT,
        u32_to_f32(end_tick) * TICK_DT,
    );
    if !t0.is_finite() || !t1.is_finite() || t1 <= t0 {
        return None;
    }
    let mut tracks = Vec::with_capacity(clip.tracks.len());
    for tr in &clip.tracks {
        let (v0, v1) = (at(tr, t0), at(tr, t1));
        let mut times = Vec::with_capacity(tr.times.len() + 2);
        let mut values = Vec::with_capacity(tr.times.len() + 2);
        times.push(0.0);
        values.push(v0);
        for t in &tr.times {
            if *t > t0 && *t < t1 {
                times.push(*t - t0);
                values.push(at(tr, *t));
            }
        }
        times.push(t1 - t0);
        values.push(v1);
        tracks.push(bake_track(times, values));
    }
    Some(bake_clip(tracks))
}

/// Scale every key time by `num / den`, rounded to the nearest tick.
///
/// `None` when `num` or `den` is 0 or the ratio leaves `1/8..=8`.
pub fn retime(clip: &QClip, num: u32, den: u32) -> Option<QClip> {
    if num == 0 || den == 0 {
        return None;
    }
    let (n, d) = (u64::from(num), u64::from(den));
    if n * 8 < d || n > d * 8 {
        return None;
    }
    let (nf, df) = (u32_to_f32(num), u32_to_f32(den));
    let mut tracks = Vec::with_capacity(clip.tracks.len());
    for tr in &clip.tracks {
        let times: Vec<f32> = tr
            .times
            .iter()
            .map(|t| (t * nf / df / TICK_DT).round() * TICK_DT)
            .collect();
        tracks.push(QTrack {
            times,
            keys: tr.keys.clone(),
            min: tr.min,
            max: tr.max,
        });
    }
    Some(bake_clip(tracks))
}

/// Drop keys the neighbours already predict within `tolerance`.
///
/// First and last keys always stay; greedy left to right. Non-finite or
/// negative tolerance reads 0 (only exact points drop).
pub fn cleanup(clip: &QClip, tolerance: f32) -> QClip {
    let tol = if tolerance.is_finite() && tolerance > 0.0 {
        tolerance
    } else {
        0.0
    };
    let tol2 = tol * tol;
    let mut tracks = Vec::with_capacity(clip.tracks.len());
    for tr in &clip.tracks {
        tracks.push(simplify(tr, tol2));
    }
    QClip {
        tracks,
        dur: clip.dur,
    }
}

/// Greedy pass of [`cleanup`] for one track; short or ragged tracks clone.
fn simplify(tr: &QTrack, tol2: f32) -> QTrack {
    let n = tr.times.len();
    if n != tr.keys.len() || n <= 2 {
        return tr.clone();
    }
    let vals: Vec<[f32; 3]> = tr.times.iter().map(|t| at(tr, *t)).collect();
    let mut kept: Vec<usize> = Vec::with_capacity(n);
    kept.push(0);
    for i in 1..n - 1 {
        let last = kept[kept.len() - 1];
        let pred = lerp3(
            vals[last],
            vals[i + 1],
            tr.times[last],
            tr.times[i + 1],
            tr.times[i],
        );
        if dist2(pred, vals[i]) > tol2 {
            kept.push(i);
        }
    }
    kept.push(n - 1);
    if kept.len() == n {
        return tr.clone();
    }
    QTrack {
        times: kept.iter().map(|i| tr.times[*i]).collect(),
        keys: kept.iter().map(|i| tr.keys[*i]).collect(),
        min: tr.min,
        max: tr.max,
    }
}

#[cfg(test)]
mod tests {
    use super::super::clip::{bake_clip, bake_track, qclip_hash, sample_track};
    use super::*;

    fn straight() -> QClip {
        let times: Vec<f32> = (0..=60).map(|k| k as f32 * TICK_DT).collect();
        let values: Vec<[f32; 3]> = (0..=60)
            .map(|k| [k as f32 * 0.1, k as f32 * 0.2, k as f32 * 0.3])
            .collect();
        bake_clip(vec![bake_track(times, values)])
    }

    fn zigzag() -> QClip {
        let times: Vec<f32> = (0..=20).map(|k| k as f32 * TICK_DT).collect();
        let values: Vec<[f32; 3]> = (0..=20)
            .map(|k| {
                if k % 2 == 0 {
                    [0.0, 0.0, 0.0]
                } else {
                    [1.0, 0.0, 0.0]
                }
            })
            .collect();
        bake_clip(vec![bake_track(times, values)])
    }

    fn probe(c: &QClip, track: usize, t: f32) -> [f32; 3] {
        let mut hint = 0usize;
        sample_track(&c.tracks[track], t, &mut hint)
    }

    #[test]
    fn trim_keeps_boundary_values() {
        let c = straight();
        let t = trim(&c, 10, 50).expect("window holds keys");
        assert!((t.dur - 40.0 * TICK_DT).abs() < 1e-6);
        assert!((t.tracks[0].times[0] - 0.0).abs() < 1e-9);
        let (a, b) = (probe(&t, 0, 0.0), probe(&c, 0, 10.0 * TICK_DT));
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-3, "{a:?} vs {b:?}");
        }
        assert!(trim(&c, 50, 10).is_none());
        assert!(trim(&c, 10, 10).is_none());
    }

    #[test]
    fn retime_half_gives_exact_ticks() {
        let c = straight();
        let half = retime(&c, 1, 2).expect("1/2 lands inside 1/8..=8");
        assert_eq!(*half.tracks[0].times.last().expect("keys"), 30.0 * TICK_DT);
        let dbl = retime(&c, 2, 1).expect("2/1 lands inside 1/8..=8");
        assert_eq!(*dbl.tracks[0].times.last().expect("keys"), 120.0 * TICK_DT);
        assert!(retime(&c, 0, 1).is_none());
        assert!(retime(&c, 1, 0).is_none());
        assert!(retime(&c, 1, 16).is_none());
        assert!(retime(&c, 16, 1).is_none());
    }

    #[test]
    fn cleanup_drops_collinear_keys() {
        let c = straight();
        let clean = cleanup(&c, 0.01);
        assert_eq!(clean.tracks[0].keys.len(), 2);
        let z = zigzag();
        let kept = cleanup(&z, 0.01);
        assert_eq!(kept.tracks[0].keys.len(), z.tracks[0].keys.len());
        for t in &c.tracks[0].times {
            let (a, b) = (probe(&clean, 0, *t), probe(&c, 0, *t));
            let d = dist2(a, b).sqrt();
            assert!(d <= 0.01 + 1e-4, "drift {d} at {t}");
        }
        let nan_kept = cleanup(&c, f32::NAN).tracks[0].keys.len();
        let zero_kept = cleanup(&c, 0.0).tracks[0].keys.len();
        assert_eq!(nan_kept, zero_kept);
        assert!(nan_kept > 2);
        assert_eq!(cleanup(&c, -1.0).tracks[0].keys.len(), zero_kept);
    }

    #[test]
    fn edits_repeat() {
        let c = straight();
        let (a, b) = (
            qclip_hash(&trim(&c, 5, 55).expect("window")),
            qclip_hash(&trim(&c, 5, 55).expect("window")),
        );
        assert_eq!(a, b);
        let (a, b) = (
            qclip_hash(&retime(&c, 1, 2).expect("ratio")),
            qclip_hash(&retime(&c, 1, 2).expect("ratio")),
        );
        assert_eq!(a, b);
        assert_eq!(
            qclip_hash(&cleanup(&c, 0.01)),
            qclip_hash(&cleanup(&c, 0.01))
        );
    }

    #[test]
    fn u32_to_f32_matches_as_cast() {
        // Oracle: `as` is the correctly-rounded reference; production code
        // must not use it (ratchet), so the helper must match bit-exact.
        for x in [
            0u32,
            1,
            2,
            3,
            60,
            255,
            256,
            1000,
            65535,
            65536,
            1_000_000,
            16_777_215,
            16_777_216,
            16_777_217,
            100_000_000,
            u32::MAX / 2,
            u32::MAX - 1,
            u32::MAX,
        ] {
            assert_eq!(
                super::u32_to_f32(x).to_bits(),
                (x as f32).to_bits(),
                "mismatch for {x}"
            );
        }
        for t in [0u32, 10, 50, 60, 120] {
            assert_eq!(
                super::u32_to_f32(t).to_bits(),
                (t as f32).to_bits(),
                "tick {t}"
            );
        }
    }
}
