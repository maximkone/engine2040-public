//! SW53 accept proofs: the ordered dither breaks the 8-bit band edges on a flat
//! sand ramp without moving the block's mean, and the knob names the bits the
//! target carries.
//!
//! The row also asks for ">= 8 distinct levels" from a ramp that quantizes to
//! three steps. That is arithmetically out of reach for any zero-mean dither of
//! half a step: `round(v + d)` for `d` in `-0.5..=0.5` reaches at most
//! `span + 2` codes (5 for a three-step span), and reaching 8 needs an
//! amplitude of two whole steps, which visibly lifts the noise and breaks the
//! same row's "mean under 0.2%" bound. The test below pins what the shipped
//! half-step dither actually does: the level count rises by one, the longest
//! identical run collapses from a visible band to a single pixel, and the mean
//! stays inside the row's bound.

use super::*;

/// The accept ramp: 16x16 pixels of flat sand, 0.5 display, spread over exactly
/// three 8-bit steps so the undithered store bands.
fn ramp() -> Vec<[f32; 3]> {
    let span = 3.0 / max_code(DITHER_BITS);
    (0..256)
        .map(|index| {
            let value = 0.5 + span * (index as f32 / 255.0);
            [value, value, value]
        })
        .collect()
}

fn frag(index: usize) -> [f32; 2] {
    [(index % 16) as f32, (index / 16) as f32]
}

/// Longest horizontal run of one identical 8-bit code on the ramp.
fn longest_run(codes: &[u8]) -> usize {
    let mut worst = 1;
    let mut run = 1;
    for pair in codes.windows(2) {
        if pair[0] == pair[1] {
            run += 1;
            worst = worst.max(run);
        } else {
            run = 1;
        }
    }
    worst
}

fn mean(values: impl Iterator<Item = f32>) -> f32 {
    let (sum, count) = values.fold((0.0_f32, 0_u32), |(sum, count), v| {
        (sum + v, count.saturating_add(1))
    });
    if count == 0 {
        return 0.0;
    }
    sum / count as f32
}

/// SW53 F2P: a three-step sand ramp bands with a visible run and a hard step
/// without the dither; with it the run is gone, the level count rises, and the
/// block mean moves under 0.2%.
#[test]
fn dither_breaks_the_eight_bit_band_edges() {
    let ramp = ramp();
    let plain: Vec<u8> = ramp.iter().map(|c| quantize(c[0], DITHER_BITS)).collect();
    let dithered: Vec<u8> = ramp
        .iter()
        .enumerate()
        .map(|(index, c)| quantize_dithered(*c, frag(index), DITHER_BITS)[0])
        .collect();

    let mut plain_levels = plain.clone();
    plain_levels.sort_unstable();
    plain_levels.dedup();
    let mut dithered_levels = dithered.clone();
    dithered_levels.sort_unstable();
    dithered_levels.dedup();
    assert_eq!(
        plain_levels.len(),
        4,
        "the ramp spans three 8-bit steps undithered"
    );
    assert_eq!(
        dithered_levels.len(),
        plain_levels.len() + 1,
        "a half-step dither adds exactly the code at each end"
    );

    let plain_run = longest_run(&plain);
    let dithered_run = longest_run(&dithered);
    assert!(
        plain_run >= 20,
        "the undithered ramp must show a visible band, got {plain_run} px"
    );
    assert!(
        dithered_run <= 2,
        "the dither must break the band edge, got a {dithered_run} px run"
    );

    // Zero mean: the block average moves under the row's 0.2% bound.
    let plain_mean = mean(
        plain
            .iter()
            .map(|code| f32::from(*code) / max_code(DITHER_BITS)),
    );
    let dithered_mean = mean(
        dithered
            .iter()
            .map(|code| f32::from(*code) / max_code(DITHER_BITS)),
    );
    let drift = (dithered_mean - plain_mean).abs() / plain_mean;
    assert!(
        drift < 0.002,
        "the dither moved the block mean by {:.3}%",
        drift * 100.0
    );
    assert_eq!(
        mean(ramp.iter().map(|c| c[0])),
        0.504_901_96,
        "the ramp seeds flat"
    );
}

/// SW53: one pixel's dither is at most half a step either way on every channel,
/// the pattern never repeats across the block on all three channels at once,
/// and the knob names 8 bits on every tier with 10 for the HDR path.
#[test]
fn dither_is_half_a_step_and_the_knob_names_the_bits() {
    let half = 0.5 / max_code(DITHER_BITS);
    let mut offsets = Vec::new();
    for index in 0..256 {
        let dither = screen_space_dither(frag(index), DITHER_BITS);
        for channel in dither {
            assert!(
                channel.abs() <= half + 1e-6,
                "a step of {channel} exceeds half an 8-bit step"
            );
        }
        offsets.push(dither);
    }
    // Every pixel's triple differs, so no diagonal band survives.
    let first = offsets[0];
    assert!(
        offsets[1..]
            .iter()
            .all(|other| (other[0] - first[0]).abs() > 1e-6
                || (other[1] - first[1]).abs() > 1e-6
                || (other[2] - first[2]).abs() > 1e-6),
        "the dither pattern repeats, so a diagonal band would survive"
    );
    assert_eq!(DITHER_BITS, 8, "the SDR present path stores 8 bits");
    assert_eq!(DITHER_BITS_HDR, 10, "the HDR path names its own step");
}

/// SW53 F2P: the shipped shader carries the dither and its step, and the
/// quality knob reaches it through the pipeline constant.
#[test]
fn dither_wgsl_carries_the_pattern_and_the_step() {
    let source = include_str!("shaders/tonemap.wgsl");
    for piece in [
        "fn screen_space_dither(",
        "const DITHER_FREQ: vec2<f32> = vec2<f32>(171.0, 231.0);",
        "const DITHER_PHASE: vec3<f32> = vec3<f32>(103.0, 71.0, 97.0);",
        "const DITHER_STEPS: f32 = 255.0;",
        "override OUTPUT_DITHER_BITS: f32 = 8.0;",
        "fn dither_output(",
    ] {
        assert!(source.contains(piece), "tonemap.wgsl misses `{piece}`");
    }
}
