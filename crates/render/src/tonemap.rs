//! Display-transform CPU mirror for the one WGSL copy plus its gate test.
//!
//! Mirrors `shaders/tonemap.wgsl` scalar math in `f32` so the hue and dither
//! gates run headless. Presentation only: never hashed, never simulated.
//! Units: linear HDR radiance in, display `0..=1` out, dither in steps of
//! one 8-bit code. Errors: none; finite input gives finite output by guard.

/// White anchor shared with the post chain, same units as linear HDR.
pub(crate) const DISPLAY_WHITE: f32 = 11.2;
/// Hard cap before the curve so huge radiance stays finite.
const PEAK_CAP: f32 = 1e6;
/// Floor under divisors so zero stays finite.
const DIV_FLOOR: f32 = 1e-6;
/// Floor under the white anchor so a broken curve cannot divide by zero.
const WHITE_FLOOR: f32 = 1e-4;
/// One 8-bit code step.
const LSB: f32 = 1.0 / 255.0;

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// Per-channel compressive fit, denominator guarded for finite output.
fn fit_one(x: f32) -> f32 {
    let v = x.max(0.0);
    let num = v * (2.51 * v + 0.03);
    let den = (v * (2.43 * v + 0.59) + 0.14).max(1e-4);
    clamp01(num / den)
}

/// Log-poly fit for the selectable branch, offset input keeps log finite.
fn analytic_one(x: f32) -> f32 {
    let v = x.max(0.0);
    let ev = (v + 0.001953125).log2();
    let t = ((ev + 6.0) / 8.0).clamp(0.0, 1.0);
    clamp01(t * t * (3.0 - 2.0 * t))
}

fn peak_of(c: [f32; 3]) -> f32 {
    c[0].max(c[1].max(c[2]))
}

fn capped(c: [f32; 3]) -> [f32; 3] {
    [
        c[0].max(0.0).min(PEAK_CAP),
        c[1].max(0.0).min(PEAK_CAP),
        c[2].max(0.0).min(PEAK_CAP),
    ]
}

/// Hue-keeping display: one scale from the per-channel curve keeps ratios.
fn hue_keep_display(c: [f32; 3]) -> [f32; 3] {
    let c = capped(c);
    let peak = peak_of(c);
    let white = fit_one(DISPLAY_WHITE).max(WHITE_FLOOR);
    let toned_peak = fit_one(peak) / white;
    let scale = toned_peak / peak.max(DIV_FLOOR);
    [c[0] * scale, c[1] * scale, c[2] * scale]
}

/// Selectable display: same one-scale shape over the log-poly curve.
fn analytic_display(c: [f32; 3]) -> [f32; 3] {
    let c = capped(c);
    let peak = peak_of(c);
    let white = analytic_one(DISPLAY_WHITE).max(WHITE_FLOOR);
    let toned_peak = analytic_one(peak) / white;
    let scale = toned_peak / peak.max(DIV_FLOOR);
    [c[0] * scale, c[1] * scale, c[2] * scale]
}

fn display_tonemap(c: [f32; 3], analytic: bool) -> [f32; 3] {
    if analytic {
        analytic_display(c)
    } else {
        hue_keep_display(c)
    }
}

/// Fixed-pattern 4x4 ordered dither scaled to one 8-bit step, signed offset.
fn dither4(x: u32, y: u32) -> f32 {
    let idx = (y & 3) * 4 + (x & 3);
    let mf = match idx {
        0 => 0.0,
        1 => 8.0,
        2 => 2.0,
        3 => 10.0,
        4 => 12.0,
        5 => 4.0,
        6 => 14.0,
        7 => 6.0,
        8 => 3.0,
        9 => 11.0,
        10 => 1.0,
        11 => 9.0,
        12 => 15.0,
        13 => 7.0,
        14 => 13.0,
        _ => 5.0,
    };
    (mf + 0.5) / 16.0 / 255.0 - 0.5 / 255.0
}

/// Standard RGB hue in degrees, `0..360`; grey returns `0`.
fn hue_deg(c: [f32; 3]) -> f32 {
    let mx = peak_of(c);
    let mn = c[0].min(c[1].min(c[2]));
    let d = mx - mn;
    if d < 1e-9 {
        return 0.0;
    }
    // `mx` is exactly one input, so equality picks the sector without drift.
    let h = if mx == c[0] {
        (c[1] - c[2]) / d
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    } * 60.0;
    if h < 0.0 {
        h + 360.0
    } else {
        h
    }
}

/// Circular hue distance in degrees.
fn hue_shift_deg(before: [f32; 3], after: [f32; 3]) -> f32 {
    let d = (hue_deg(after) - hue_deg(before)).abs();
    d.min(360.0 - d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Torch-flame orange fixture, linear HDR radiance.
    const TORCH_ORANGE: [f32; 3] = [3.0, 1.2, 0.35];

    #[test]
    fn tonemap_one_source_hue() {
        // One ACES source stays in the owned WGSL copy.
        let wgsl = include_str!("shaders/tonemap.wgsl");
        let aces_defs = wgsl.matches("fn aces").count();
        assert_eq!(aces_defs, 1, "one `fn aces` definition, got {aces_defs}");
        // RL34 slice 1: post hunk shares the one source, so post carries
        // no ACES copy and the tree total steps down toward one.
        let post = include_str!("post.rs");
        assert!(!post.contains("fn aces"), "post still carries its copy");
        let total = post.matches("fn aces").count()
            + include_str!("v1.rs").matches("fn aces").count()
            + include_str!("shadow_post.rs").matches("fn aces").count()
            + include_str!("shaders/post/post_chain.wgsl")
                .matches("fn aces")
                .count()
            + include_str!("shaders/post/composite_compute.wgsl")
                .matches("fn aces")
                .count()
            + aces_defs;
        assert_eq!(
            total, 4,
            "one source: total {total}, want 4 with post clean"
        );
        assert!(
            wgsl.contains("fn rrt_odt_fit") && wgsl.contains("fn aces_fitted"),
            "keeps the single fitted source"
        );
        assert!(
            wgsl.contains("fn dither4"),
            "keeps the fixed-pattern helper"
        );
        // Torch hue holds through the mirrored helper on both branches.
        for analytic in [false, true] {
            let out = display_tonemap(TORCH_ORANGE, analytic);
            assert!(
                out.iter().all(|c| c.is_finite()),
                "finite out for torch orange, got {out:?}"
            );
            let shift = hue_shift_deg(TORCH_ORANGE, out);
            assert!(
                shift <= 2.0,
                "torch hue shift {shift:.3} deg over 2 deg (analytic={analytic})"
            );
        }
        // White anchor maps grey white to one.
        let white = hue_keep_display([DISPLAY_WHITE; 3]);
        for c in white {
            assert!(
                (c - 1.0).abs() < 1e-5,
                "grey white maps to one, got {white:?}"
            );
        }
        // Finite stays finite across the working range on both branches.
        for px in [
            [0.0, 0.0, 0.0],
            [0.002, 0.001, 0.0005],
            [0.18, 0.18, 0.18],
            [1.0, 0.55, 0.22],
            TORCH_ORANGE,
            [11.2, 5.0, 1.0],
            [1e5, 2.0, 0.5],
        ] {
            for analytic in [false, true] {
                let out = display_tonemap(px, analytic);
                assert!(
                    out.iter().all(|c| c.is_finite()),
                    "finite for {px:?}, got {out:?}"
                );
                assert!(
                    out.iter().all(|c| *c >= 0.0),
                    "no negative display for {px:?}, got {out:?}"
                );
            }
        }
        // Dither stays within one step with a 4x4 period.
        for y in 0..8 {
            for x in 0..8 {
                let v = dither4(x, y);
                assert!(v.is_finite(), "dither finite at {x},{y}");
                let unsigned = v + 0.5 / 255.0;
                assert!(
                    (-1e-9..=LSB + 1e-9).contains(&unsigned),
                    "dither {v} at {x},{y} outside one step"
                );
                assert!(
                    (v - dither4(x & 3, y & 3)).abs() < 1e-9,
                    "4x4 period breaks at {x},{y}"
                );
                assert!(
                    (v - dither4(x + 4, y)).abs() < 1e-9 && (v - dither4(x, y + 4)).abs() < 1e-9,
                    "4x4 repeat breaks at {x},{y}"
                );
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for y in 0..4 {
            for x in 0..4 {
                let bits = dither4(x, y).to_bits();
                seen.insert(bits);
            }
        }
        assert_eq!(seen.len(), 16, "4x4 pattern holds 16 levels");
    }
}
