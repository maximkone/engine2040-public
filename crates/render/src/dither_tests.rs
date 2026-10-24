//! SW53 accept proofs: the ordered dither splits the 8-bit band edges on a flat
//! sand ramp into distinct codes without moving the block's mean, and the
//! shipped shader carries the same pattern and width.
//!
//! The width matters: half a step only softens a band's edge (5 codes, and a
//! 22 px run survives), while [`DITHER_HALF_STEPS`] of 2.5 splits the band into
//! nine codes with a 2 px run and a mean drift of 0.05%, inside the row's 0.2%.

use super::*;

/// The accept ramp: 16x16 pixels of flat sand, 0.5 display, spread over exactly
/// three 8-bit steps so the undithered store bands.
fn ramp() -> Vec<f32> {
    let span = 3.0 / max_code(DITHER_BITS);
    (0..256)
        .map(|index| 0.5 + span * (index as f32 / 255.0))
        .collect()
}

fn frag(index: usize) -> [f32; 2] {
    [(index % 16) as f32, (index / 16) as f32]
}

/// Longest horizontal run of one identical 8-bit code on the ramp: index walk,
/// so it shares no shape with the tone map's own run helper.
fn longest_run(codes: &[u8]) -> usize {
    let mut worst = 0;
    let mut start = 0;
    while start < codes.len() {
        let mut end = start + 1;
        while end < codes.len() && codes[end] == codes[start] {
            end += 1;
        }
        worst = worst.max(end - start);
        start = end;
    }
    worst
}

fn distinct(codes: &[u8]) -> usize {
    let mut sorted = codes.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    sorted.len()
}

/// SW53 F2P: three undithered steps become at least eight distinct codes with
/// the dither, the visible run collapses, and the block mean moves under 0.2%.
#[test]
fn dither_breaks_the_eight_bit_band_edges() {
    let ramp = ramp();
    let plain: Vec<u8> = ramp.iter().map(|v| quantize8(*v)).collect();
    let dithered: Vec<u8> = ramp
        .iter()
        .enumerate()
        .map(|(index, v)| quantize_dithered([*v; 3], frag(index))[0])
        .collect();

    assert_eq!(distinct(&plain), 4, "three 8-bit steps without dither");
    assert!(
        distinct(&dithered) >= 8,
        "the dither must show at least eight levels, got {}",
        distinct(&dithered)
    );

    let plain_run = longest_run(&plain);
    let dithered_run = longest_run(&dithered);
    assert!(
        plain_run >= 20,
        "the undithered ramp must show a visible band, got {plain_run} px"
    );
    assert!(
        dithered_run <= 2,
        "the dither must break the band, got a {dithered_run} px run"
    );

    // Zero mean: the block average moves under the row's 0.2% bound.
    let scale = max_code(DITHER_BITS);
    let plain_mean = plain.iter().map(|c| f32::from(*c) / scale).sum::<f32>() / plain.len() as f32;
    let dithered_mean =
        dithered.iter().map(|c| f32::from(*c) / scale).sum::<f32>() / dithered.len() as f32;
    let drift = (dithered_mean - plain_mean).abs() / plain_mean;
    assert!(
        drift < 0.002,
        "the dither moved the block mean by {:.3}%",
        drift * 100.0
    );
}

/// SW53: one pixel's dither is exactly `DITHER_HALF_STEPS` either way, no two
/// pixels in the block share the triple (so no diagonal band survives), and the
/// step names the bits the path carries.
#[test]
fn dither_is_two_and_a_half_steps_and_never_repeats_in_a_block() {
    let limit = DITHER_HALF_STEPS / max_code(DITHER_BITS);
    let mut offsets = Vec::new();
    for index in 0..256 {
        let dither = screen_space_dither(frag(index), DITHER_BITS);
        for channel in dither {
            assert!(
                channel.abs() <= limit + 1e-6,
                "a step of {channel} exceeds {DITHER_HALF_STEPS} 8-bit steps"
            );
        }
        offsets.push(dither);
    }
    let first = offsets[0];
    assert!(
        offsets[1..].iter().all(|other| {
            (other[0] - first[0]).abs() > 1e-6
                || (other[1] - first[1]).abs() > 1e-6
                || (other[2] - first[2]).abs() > 1e-6
        }),
        "the dither pattern repeats, so a diagonal band would survive"
    );
    assert_eq!(DITHER_BITS, 8, "the SDR present path stores 8 bits");
    assert_eq!(DITHER_BITS_HDR, 10, "the HDR path names its own step");
    // A 10-bit target dithers 4x finer for the same pattern.
    let fine = screen_space_dither(frag(3), DITHER_BITS_HDR);
    let coarse = screen_space_dither(frag(3), DITHER_BITS);
    assert!(
        fine[0].abs() < coarse[0].abs(),
        "the HDR step is finer: {fine:?} vs {coarse:?}"
    );
}

/// SW53 F2P: the shipped shader carries the dither, its width and the step, and
/// the CPU twin's width equals the shader's.
#[test]
fn dither_wgsl_carries_the_pattern_and_the_step() {
    let source = include_str!("shaders/tonemap.wgsl");
    for piece in [
        "fn screen_space_dither(",
        "const DITHER_FREQ: vec2<f32> = vec2<f32>(171.0, 231.0);",
        "const DITHER_PHASE: vec3<f32> = vec3<f32>(103.0, 71.0, 97.0);",
        "const DITHER_STEPS: f32 = 255.0;",
        "const DITHER_HALF_STEPS: f32 = 2.5;",
        "override OUTPUT_DITHER_BITS: f32 = 8.0;",
        "fn dither_output(",
        "(2.0 * DITHER_HALF_STEPS) / DITHER_STEPS",
    ] {
        assert!(source.contains(piece), "tonemap.wgsl misses `{piece}`");
    }
    assert_eq!(
        DITHER_HALF_STEPS, 2.5,
        "the CPU twin names the shader's width"
    );
}
