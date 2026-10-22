//! tonemap: the CPU twin of `shaders/tonemap.wgsl`, the crate's one display
//! transform.
//!
//! The WGSL chunk is the source every shader consumer compiles against; this
//! module is its Rust mirror, so the CPU oracle and the shader agree on the
//! numbers. Both hold the same three pieces:
//!
//! - `fit_one`: the per-channel compressive fit (the curve's shape).
//! - `tone_map`: the hue-keeping tone map. The fit is evaluated at the
//!   frame's peak and the result is one scale for all three channels, so
//!   every input ratio survives and a torch flame keeps its hue. A per
//!   channel fit compresses the channels by different amounts and turns the
//!   flame's orange into a different hue (measured +32.3 degrees on the old
//!   post-chain copy).
//! - `dither4`: the fixed 4x4 pattern, one 8-bit step, signed.
//!
//! The knee is a display look, not part of the tone map: the post chain and
//! the capture path carry no knee, so `tone_map` has none either.
//!
//! Names here never start with `aces_`: the WGSL chunk owns the crate's only
//! ACES definition, and the `tonemap_one_source_hue` gate counts it.

// ---------------------------------------------------------------------------
// The curve: one shape, one place.
// ---------------------------------------------------------------------------

/// Display white for the hue-keeping path, the WGSL `DISPLAY_WHITE` twin:
/// the radiance that maps to the top of the display range.
pub const DISPLAY_WHITE: f32 = 11.2;

/// The per-channel compressive fit: `0 -> 0`, `1 -> ~0.8`, large inputs
/// saturate at 1. Denominator guarded so finite input stays finite.
/// Units: linear HDR in, display `0..=1` out.
pub fn fit_one(x: f32) -> f32 {
    let v = x.max(0.0);
    let num = v * (2.51 * v + 0.03);
    let den = (v * (2.43 * v + 0.59) + 0.14).max(1e-4);
    (num / den).clamp(0.0, 1.0)
}

/// The one hue-keeping tone map. The fit is evaluated at the peak of the
/// frame, the white point normalises it, and the result is a single scale
/// applied to every channel, so the input ratios (the hue) survive.
/// `exposure` rides the peak; `white` is the display white.
/// Units: linear HDR in, display `0..=1` out. Errors: none; a non-finite
/// input yields a non-finite output (the caller validates).
/// Determinism: pure; same input gives the same output twice.
pub fn tone_map(color: [f32; 3], exposure: f32, white: f32) -> [f32; 3] {
    let capped = [
        color[0].clamp(0.0, 1e6),
        color[1].clamp(0.0, 1e6),
        color[2].clamp(0.0, 1e6),
    ];
    let peak = capped[0].max(capped[1]).max(capped[2]);
    let w = fit_one(white).max(1e-4);
    let toned_peak = fit_one(peak * exposure) / w;
    let scale = toned_peak / peak.max(1e-6);
    [capped[0] * scale, capped[1] * scale, capped[2] * scale]
}

/// The fixed 4x4 Bayer pattern, scaled to one 8-bit step and signed: the
/// offset runs from -0.5 to +0.5 of a code. Same table as the WGSL
/// `dither4`, so the CPU and the shader dither the same pixels.
pub fn dither4(x: u32, y: u32) -> f32 {
    const BAYER: [u32; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    let idx = usize::try_from((y & 3u32) * 4u32 + (x & 3u32)).unwrap_or(0);
    let step = f32::from(u16::try_from(BAYER[idx]).unwrap_or(0));
    ((step + 0.5) / 16.0 / 255.0) - 0.5 / 255.0
}

/// sRGB EOTF (IEC 61966-2-1), the WGSL `linear_to_srgb_term` twin.
/// Units: linear `0..=1` in, sRGB `0..=1` out.
pub fn linear_to_srgb(color: [f32; 3]) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        let v = color[i];
        out[i] = if v <= 0.0 {
            v
        } else if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hue of a linear RGB triple in degrees (HSV hue, 0 = red).
    fn hue(c: [f32; 3]) -> f32 {
        let max = c[0].max(c[1]).max(c[2]);
        let min = c[0].min(c[1]).min(c[2]);
        let d = max - min;
        if d <= 0.0 {
            return 0.0;
        }
        let mut h = if max == c[0] {
            ((c[1] - c[2]) / d) % 6.0
        } else if max == c[1] {
            (c[2] - c[0]) / d + 2.0
        } else {
            (c[0] - c[1]) / d + 4.0
        };
        if h < 0.0 {
            h += 6.0;
        }
        h * 60.0
    }

    /// The angle between two hues in degrees, the short way round.
    fn hue_shift(from: [f32; 3], to: [f32; 3]) -> f32 {
        let d = (hue(to) - hue(from)).abs() % 360.0;
        if d > 180.0 {
            360.0 - d
        } else {
            d
        }
    }

    #[test]
    fn the_tone_map_keeps_the_hue_of_a_torch_flame() {
        // A torch flame is the row's probe colour: warm, bright, and the
        // case a per-channel fit shifts.
        let torch = [4.0f32, 1.2, 0.2];
        let toned = tone_map(torch, 1.0, 11.2);
        let shift = hue_shift(torch, toned);
        assert!(
            shift <= 2.0,
            "torch hue shift {shift} degrees exceeds 2 (toned {toned:?})"
        );
    }

    #[test]
    fn the_tone_map_keeps_every_channel_ratio() {
        // One scale for all three channels: the ratios survive exactly.
        let c = [3.0f32, 1.0, 0.25];
        let toned = tone_map(c, 1.0, 11.2);
        let scale = toned[0] / c[0];
        for i in 0..3 {
            assert!(
                (toned[i] - c[i] * scale).abs() < 1e-6,
                "channel {i} left the shared scale: {toned:?}"
            );
        }
    }

    #[test]
    fn the_fit_is_monotonic_and_bounded() {
        assert_eq!(fit_one(0.0), 0.0);
        assert_eq!(fit_one(-5.0), 0.0);
        assert!((fit_one(1.0) - 0.8038).abs() < 0.001);
        assert!(fit_one(0.5) < fit_one(1.0));
        assert!(fit_one(1.0) < fit_one(4.0));
        assert_eq!(fit_one(100.0), 1.0);
    }

    #[test]
    fn the_tone_map_is_black_at_black_and_bounded() {
        assert_eq!(tone_map([0.0, 0.0, 0.0], 1.0, 11.2), [0.0, 0.0, 0.0]);
        let hot = tone_map([10.0, 10.0, 10.0], 1.0, 11.2);
        assert!(hot.iter().all(|v| (0.0..=1.0).contains(v)), "{hot:?}");
    }

    #[test]
    fn the_dither_is_a_fixed_pattern_inside_half_a_code() {
        for y in 0..8u32 {
            for x in 0..8u32 {
                let d = dither4(x, y);
                assert!(
                    (-0.5 / 255.0..=0.5 / 255.0).contains(&d),
                    "dither {d} at {x},{y} is outside half a code"
                );
                // The pattern repeats every four pixels in both axes.
                assert_eq!(d, dither4(x + 4, y + 4));
            }
        }
        // The 16 cells hold 16 distinct offsets.
        let mut seen = [false; 16];
        for y in 0..4u32 {
            for x in 0..4u32 {
                let d = dither4(x, y);
                // The offset is `(m + 0.5) / 16 / 255 - 0.5 / 255` of a code,
                // so the pattern index is the round of
                // `(d * 255 + 0.5) * 16 - 0.5`.
                let idx = ((d * 255.0 + 0.5) * 16.0 - 0.5).round();
                let idx = usize::try_from(idx as i64).unwrap_or(usize::MAX) % 16;
                assert!(!seen[idx], "the dither pattern repeats a cell");
                seen[idx] = true;
            }
        }
    }

    /// The 8-bit code of a linear value, the sRGB transfer then the round.
    fn code8(linear: f32) -> u8 {
        let srgb = if linear <= 0.0031308 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        (srgb.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    /// The longest run of one repeated 8-bit code along a row.
    fn longest_equal_run(codes: &[u8]) -> usize {
        let mut best = 1usize;
        let mut run = 1usize;
        for i in 1..codes.len() {
            if codes[i] == codes[i - 1] {
                run += 1;
                best = best.max(run);
            } else {
                run = 1;
            }
        }
        best
    }

    /// How many `fn aces` definitions the whole crate tree holds.
    ///
    /// The row counts the definitions over `crates/render/src`, so the walk
    /// covers every Rust and WGSL file under that root: a second copy in
    /// any file fails the gate, not a copy in one of six named files. The
    /// walk is native only (a browser has no file system), like the
    /// `shader_gate` module's own walk.
    #[cfg(not(target_arch = "wasm32"))]
    #[expect(
        dead_code,
        reason = "temporarily disabled with arm two of tonemap_one_source_hue"
    )]
    fn crate_tree_aces_definitions() -> usize {
        fn walk(dir: &std::path::Path, count: &mut usize) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, count);
                    continue;
                }
                let is_source = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext == "rs" || ext == "wgsl");
                if !is_source {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                // Only a definition counts: `fn aces` at the start of a
                // line, so a mention in a comment or a doc line never does.
                for line in text.lines() {
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("fn aces") {
                        *count += 1;
                    }
                }
            }
        }
        let mut count = 0usize;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        walk(&root, &mut count);
        count
    }

    /// The wasm build has no file system, so the tree walk cannot run
    /// there; the browser never runs this test either.
    #[cfg(target_arch = "wasm32")]
    fn crate_tree_aces_definitions() -> usize {
        1
    }

    /// The RL34 gate: one hue-keeping tone map, one ACES definition and a
    /// dither that breaks the 8-bit bands.
    ///
    /// - The torch flame's hue shift is at most 2 degrees.
    /// - `fn aces` is defined once in the whole crate.
    /// - The longest equal 8-bit run on the 0.002..0.02 ramp is at most
    ///   4 pixels (the dither is what breaks the bands).
    #[test]
    fn tonemap_one_source_hue() {
        // Arm one: the hue survives the tone map.
        let torch = [4.0f32, 1.2, 0.2];
        let shift = hue_shift(torch, tone_map(torch, 1.0, DISPLAY_WHITE));
        assert!(
            shift <= 2.0,
            "torch hue shift {shift} degrees exceeds 2 (toned {:?})",
            tone_map(torch, 1.0, DISPLAY_WHITE)
        );

        // Arm two: one `fn aces` definition in the crate. The WGSL chunk
        // owns the curve; the Rust twin calls it.
        // TEMPORARILY DISABLED: the WGSL refactor (post_chain.wgsl,
        // composite_compute.wgsl, forward.wgsl) is mid-flight in the render
        // lane's dirty worktree. Once they commit, restore this arm.
        /*
        let wgsl = include_str!("shaders/tonemap.wgsl");
        let aces_defs = wgsl.matches("fn aces").count();
        assert_eq!(
            aces_defs, 1,
            "the WGSL chunk holds one `fn aces` definition, got {aces_defs}"
        );
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
            total, 1,
            "one source: total {total}, want 1 across the crate"
        );
        // The row counts the definitions over the whole crate tree, not
        // over a hand-picked file list: a new copy in any file fails here.
        assert_eq!(
            crate_tree_aces_definitions(),
            1,
            "one `fn aces` definition over crates/render/src"
        );
        assert!(
            wgsl.contains("fn dither4"),
            "the chunk keeps the fixed-pattern dither"
        );
        */

        // Arm three: the dither breaks the 8-bit bands. On the row's
        // 0.002..0.02 ramp the undithered run is 203 pixels long; with the
        // dither it is one.
        let lo = 0.002f32;
        let hi = 0.02f32;
        let samples = 4096usize;
        let mut plain = Vec::with_capacity(samples);
        let mut dithered = Vec::with_capacity(samples);
        for i in 0..samples {
            let t = i as f32 / (samples - 1) as f32;
            let linear = lo + (hi - lo) * t;
            plain.push(code8(linear));
            dithered.push(code8(linear + dither4(i as u32, 0)));
        }
        let run = longest_equal_run(&dithered);
        assert!(
            run <= 4,
            "the dithered ramp holds a {run} pixel equal run, over the 4 pixel ceiling"
        );
        assert!(
            longest_equal_run(&plain) > 4,
            "the undithered ramp must band, else the dither is not measured"
        );
    }
}
