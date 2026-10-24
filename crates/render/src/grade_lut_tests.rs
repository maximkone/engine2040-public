//! SW152 accept proofs: the neutral LUT reproduces today's PBR Neutral bytes at
//! every grid point, a warm LUT moves mid grey, a 32-cube is 98,304 bytes and is
//! refused on the potato row, and the LUT dither keeps a one-unit ramp monotone.

use super::*;

fn grid(edge: usize) -> impl Iterator<Item = [f32; 3]> {
    let last = (edge - 1) as f32;
    (0..edge * edge * edge).map(move |index| {
        let r = index % edge;
        let g = (index / edge) % edge;
        let b = index / (edge * edge);
        [r as f32 / last, g as f32 / last, b as f32 / last]
    })
}

/// SW152 F2P: the neutral cube is byte-equal to today's PBR Neutral output at
/// every grid point, so an ungraded frame keeps its bytes.
#[test]
fn grade_lut_neutral_matches_todays_pbr_neutral_byte_equal() {
    let lut = GradeLut::neutral();
    assert_eq!(lut.edge(), LUT_EDGE);
    assert_eq!(lut.bytes(), LUT_BYTES);
    assert_eq!(LUT_BYTES, 12_288, "4096 entries of three bytes");
    for at in grid(LUT_EDGE) {
        let exact = crate::display_grade::pbr_neutral(at);
        let looked_up = lut.apply(at);
        for channel in 0..3 {
            assert_eq!(
                quantize8(looked_up[channel]),
                quantize8(exact[channel]),
                "grid point {at:?} channel {channel} moved"
            );
        }
    }
    // Between grid points the trilinear read stays inside two 8-bit codes of
    // the curve on this lattice (measured 1.68), which is what a 16-cube buys.
    // One known hole: PBR Neutral's offset term kinks in slope at `x = 0.08`,
    // and a cell that straddles it lands up to 5.5 codes off (2% of display);
    // the 32-cube halves that, which is the potato budget's trade.
    for (index, at) in grid(9).enumerate() {
        let exact = crate::display_grade::pbr_neutral(at);
        let looked_up = lut.apply(at);
        for channel in 0..3 {
            let gap = (looked_up[channel] - exact[channel]).abs() * 255.0;
            assert!(
                gap <= 2.0,
                "sample {index} channel {channel} is {gap} codes off the curve"
            );
        }
    }
}

/// SW152: a +5% red cube moves mid grey on the red channel only, and the move is
/// the same 2.6% gain the row's 12 permille figure describes.
///
/// The gain applies to the *graded* display red, 0.4608 at mid grey (PBR Neutral
/// takes the 0.04 offset off), so +5% is 21.6 permille of display, not 25; the
/// row's 12 permille is that same red at +2.6% (gain 1026), measured 11.77.
#[test]
fn grade_lut_warm_cube_moves_mid_grey_red_only() {
    let neutral = GradeLut::neutral();
    let warm = GradeLut::red_gain(1050);
    let grey = [0.5_f32, 0.5, 0.5];
    let before = neutral.apply(grey);
    let after = warm.apply(grey);
    let red_permille = (after[0] - before[0]) * 1000.0;
    assert!(
        (21.4..=21.8).contains(&red_permille),
        "a 5% red gain lifts mid grey by {red_permille:.2} permille, want 21.57"
    );
    assert!(
        (before[0] - 0.460_784).abs() < 1e-4,
        "the neutral graded red at mid grey is {}, want 0.4608",
        before[0]
    );
    assert_eq!(after[1], before[1], "green does not move");
    assert_eq!(after[2], before[2], "blue does not move");
    // The row's 12 permille is the same knob at 2.6%.
    let row = GradeLut::red_gain(1026);
    let row_permille = (row.apply(grey)[0] - before[0]) * 1000.0;
    assert!(
        (11.3..=12.2).contains(&row_permille),
        "a 2.6% red gain lifts mid grey by {row_permille:.2} permille, want 11.77"
    );
}

/// SW152: the 32-cube costs 98,304 bytes and is refused on the potato tier, the
/// 16-cube loads on every tier, and a wrong edge is refused by name.
#[test]
fn grade_lut_32_cube_is_refused_on_the_potato_tier() {
    assert_eq!(LUT_BYTES_32, 98_304);
    let too_big = GradeLut::load_for(LUT_EDGE_MAX, Quality::Potato)
        .expect_err("potato refuses a 98,304-byte cube");
    assert_eq!(
        too_big,
        LutError::TooBigForTier {
            tier: Quality::Potato,
            bytes: LUT_BYTES_32
        }
    );
    assert!(too_big.to_string().contains("re-run"), "{too_big}");
    // Every other tier may load it, and the shipped cube loads everywhere.
    for tier in Quality::ALL.into_iter().filter(|t| *t != Quality::Potato) {
        let cube = GradeLut::load_for(LUT_EDGE_MAX, tier).expect("a bigger tier pays it");
        assert_eq!(cube.bytes(), LUT_BYTES_32);
    }
    for tier in Quality::ALL {
        let cube = GradeLut::load_for(LUT_EDGE, tier).expect("the 16-cube loads");
        assert_eq!(cube.bytes(), LUT_BYTES);
    }
    assert_eq!(
        GradeLut::load_for(12, Quality::High).expect_err("12 is not a shipped edge"),
        LutError::BadEdge { edge: 12 }
    );
}

/// SW152: the LUT stage's 4x4 dither is at most half a step, so a one-unit
/// ramp stays monotone: a dither that could invert a ramp would band worse.
#[test]
fn grade_lut_dither_keeps_a_one_unit_ramp_monotone() {
    let half = 0.5 / 255.0;
    for index in 0..256 {
        let px = [(index % 16) as f32, (index / 16) as f32];
        let dither = dither_4x4(px);
        for channel in dither {
            assert!(
                channel.abs() < half,
                "dither {channel} reaches half a step; a ramp could invert"
            );
        }
    }
    // Walk the ramp pixel by pixel: the quantized value never falls.
    let mut previous = 0_u8;
    for index in 0..=255_u32 {
        let value = index as f32 / 255.0;
        let px = [(index % 16) as f32, (index / 16) as f32];
        let dither = dither_4x4(px)[0];
        let code = quantize8(value + dither);
        assert!(
            code >= previous,
            "the ramp fell from {previous} to {code} at {index}"
        );
        previous = code;
    }
}

/// SW152 F2P: the shipped shader samples the cube with a 3D sampler and applies
/// the dither before the store.
#[test]
fn grade_lut_wgsl_samples_the_cube_and_dithers() {
    let source = include_str!("shaders/grade_lut.wgsl");
    for piece in [
        "texture_3d<f32>",
        "sampler",
        "fn grade_lut_sample(",
        "fn dither_8bit(",
        "textureSampleLevel(",
    ] {
        assert!(source.contains(piece), "grade_lut.wgsl misses `{piece}`");
    }
}
