//! Tests for the parent module.

use super::*;
use crate::post::{apply_grade, tonemap_aces, GradeConfig};

const GOLDEN: FrameTolerance = FrameTolerance::GOLDEN;

/// A flat mid-grey RGBA frame of `pixels` pixels.
fn flat(pixels: usize) -> Vec<u8> {
    [100, 100, 100, 255].repeat(pixels)
}

/// `frame` with channel `channel` of pixel `pixel` raised by `delta`.
fn bump(frame: &mut [u8], pixel: usize, channel: usize, delta: u8) {
    frame[pixel * 4 + channel] += delta;
}

fn verdict(golden: &[u8], frame: &[u8]) -> FrameVerdict {
    frames_match(golden, frame, &GOLDEN).expect("same-size RGBA frames compare")
}

#[test]
fn identical_frames_match_with_both_numbers_zero() {
    let golden = flat(64);
    let v = verdict(&golden, &golden);
    assert!(v.matches(), "{}", v.describe());
    assert_eq!(
        (v.mean_abs_milli, v.over_pixels, v.over_ppm, v.worst),
        (0, 0, 0, 0)
    );
}

#[test]
fn mean_limit_is_exactly_one_and_a_half_levels() {
    // 10 pixels = 30 channels. 15 channels off by 2 and 15 by 1 sum to 45:
    // mean exactly 1.5.
    let golden = flat(10);
    let mut frame = golden.clone();
    for channel in 0..30usize {
        let delta = if channel < 15 { 2 } else { 1 };
        bump(&mut frame, channel / 3, channel % 3, delta);
    }
    let at_limit = verdict(&golden, &frame);
    assert_eq!(at_limit.mean_abs_milli, 1500);
    assert!(
        at_limit.mean_ok && at_limit.matches(),
        "{}",
        at_limit.describe()
    );
    bump(&mut frame, 9, 2, 1);
    let over = verdict(&golden, &frame);
    assert!(!over.mean_ok && !over.matches(), "{}", over.describe());
    assert!(over.over_ok, "no pixel is far off, only the mean broke");
}

#[test]
fn far_off_share_limit_is_exactly_half_a_percent_over_twelve_levels() {
    let golden = flat(1000);
    let mut frame = golden.clone();
    for pixel in 0..5 {
        bump(&mut frame, pixel, 0, 13);
    }
    let at_limit = verdict(&golden, &frame);
    assert_eq!((at_limit.over_pixels, at_limit.over_ppm), (5, 5000));
    assert!(at_limit.matches(), "{}", at_limit.describe());
    bump(&mut frame, 5, 1, 13);
    let over = verdict(&golden, &frame);
    assert!(!over.over_ok && over.mean_ok, "{}", over.describe());
    assert!(!over.matches());
}

#[test]
fn twelve_levels_is_not_far_off_but_thirteen_is() {
    let golden = flat(1000);
    let mut at_twelve = golden.clone();
    let mut at_thirteen = golden.clone();
    for pixel in 0..10 {
        bump(&mut at_twelve, pixel, 0, 12);
        bump(&mut at_thirteen, pixel, 0, 13);
    }
    let twelve = verdict(&golden, &at_twelve);
    assert_eq!((twelve.over_pixels, twelve.worst), (0, 12));
    assert!(twelve.matches(), "{}", twelve.describe());
    let thirteen = verdict(&golden, &at_thirteen);
    assert_eq!((thirteen.over_pixels, thirteen.worst), (10, 13));
    assert!(!thirteen.over_ok, "1% of pixels over 12 breaks the share");
}

#[test]
fn each_number_catches_what_the_other_misses() {
    // Whole-frame drift of 2 levels: no pixel is far off, the mean breaks.
    let golden = flat(1000);
    let mut drift = golden.clone();
    for byte in drift.chunks_exact_mut(4) {
        byte[..3].iter_mut().for_each(|c| *c += 2);
    }
    let drift_v = verdict(&golden, &drift);
    assert_eq!(drift_v.over_pixels, 0);
    assert!(
        drift_v.over_ok && !drift_v.mean_ok,
        "{}",
        drift_v.describe()
    );
    // Sparse spikes: 0.6% of pixels off by 100. The mean stays tiny, the
    // share breaks.
    let mut spikes = golden.clone();
    for pixel in 0..6 {
        bump(&mut spikes, pixel, 1, 100);
    }
    let spikes_v = verdict(&golden, &spikes);
    assert!(
        spikes_v.mean_ok && !spikes_v.over_ok,
        "{}",
        spikes_v.describe()
    );
}

#[test]
fn alpha_is_ignored() {
    let golden = flat(8);
    let mut frame = golden.clone();
    for pixel in 0..8 {
        frame[pixel * 4 + 3] = 0;
    }
    let v = verdict(&golden, &frame);
    assert_eq!((v.mean_abs_milli, v.worst), (0, 0));
}

#[test]
fn ragged_frames_are_typed_errors() {
    let ok = flat(4);
    assert_eq!(
        frames_match(&ok, &flat(3), &GOLDEN),
        Err(FrameError::LengthMismatch {
            golden: 16,
            frame: 12
        })
    );
    assert_eq!(frames_match(&[], &[], &GOLDEN), Err(FrameError::Empty));
    assert_eq!(
        frames_match(&[0; 6], &[0; 6], &GOLDEN),
        Err(FrameError::NotRgba { len: 6 })
    );
    assert!(FrameError::NotRgba { len: 6 }.to_string().contains("6"));
}

#[test]
fn describe_names_both_numbers_and_the_broken_rule() {
    let golden = flat(1000);
    let mut drift = golden.clone();
    for byte in drift.chunks_exact_mut(4) {
        byte[0] += 3;
    }
    let v = verdict(&golden, &drift);
    assert_eq!(
        v.describe(),
        "mean_abs=1.000 (max 1.500) over12=0.0000% (max 0.5000%) worst=3 PASS"
    );
    for byte in drift.chunks_exact_mut(4) {
        byte[1] += 3;
        byte[2] += 3;
    }
    assert_eq!(
        verdict(&golden, &drift).describe(),
        "mean_abs=3.000 (max 1.500) over12=0.0000% (max 0.5000%) worst=3 FAIL(mean)"
    );
}

#[test]
fn the_skin_parity_rule_is_the_golden_rule_at_three_levels() {
    // The GPU-skinned mesh parity test held "under 0.5% of pixels differ by
    // more than 3 levels"; it now asks for exactly that through the golden
    // rule with the far-off level kept at 3.
    let rule = FrameTolerance {
        over_level: 3,
        ..FrameTolerance::GOLDEN
    };
    let golden = flat(1000);
    let mut frame = golden.clone();
    for pixel in 0..5 {
        bump(&mut frame, pixel, 0, 4);
    }
    assert!(frames_match(&golden, &frame, &rule).unwrap().matches());
    bump(&mut frame, 5, 0, 4);
    assert!(!frames_match(&golden, &frame, &rule).unwrap().matches());
    // The same frame is a pass under the looser golden level.
    assert!(frames_match(&golden, &frame, &GOLDEN).unwrap().matches());
}

/// Display value 0..1 to a byte, rounded.
fn byte_of(value: f32) -> u8 {
    let scaled = f64::from(value.clamp(0.0, 1.0)) * 255.0 + 0.5;
    u8::try_from(engine_core::checked::saturating_f64_to_u64(scaled)).unwrap_or(u8::MAX)
}

/// A 64x48 HDR scene (a warm log ramp, brighter toward the top) pushed
/// through the CPU post chain's tonemap and grade, as RGBA bytes.
fn graded_scene(grade: &GradeConfig) -> Vec<u8> {
    let (w, h) = (64u16, 48u16);
    let mut out = Vec::with_capacity(usize::from(w) * usize::from(h) * 4);
    for y in 0..h {
        for x in 0..w {
            let t = f32::from(x) / f32::from(w - 1);
            let lift = 0.6 + 0.4 * (1.0 - f32::from(y) / f32::from(h - 1));
            let luma = 0.02 * 200f32.powf(t) * lift;
            let hdr = [luma, luma * 0.85, luma * 0.6];
            let shown = tonemap_aces(hdr, 1.0, 11.2).expect("finite scene tonemaps");
            let graded = apply_grade(shown, grade).expect("default-range grade applies");
            out.extend(graded.map(byte_of));
            out.push(255);
        }
    }
    out
}

#[test]
fn the_gate_fails_when_one_post_coefficient_moves_by_a_fiftieth() {
    let golden = graded_scene(&GradeConfig::default());
    assert!(
        verdict(&golden, &graded_scene(&GradeConfig::default())).matches(),
        "the same constants reproduce the golden"
    );
    // Gain 1.00 -> 1.02 (the donor's 0.02 nudge): brighter everywhere.
    let nudged = GradeConfig {
        gain: [1.02; 3],
        ..GradeConfig::default()
    };
    let broken = verdict(&golden, &graded_scene(&nudged));
    assert!(
        !broken.matches(),
        "a 0.02 gain nudge must fail: {}",
        broken.describe()
    );
    assert!(
        !broken.mean_ok && broken.mean_abs_milli > 1500,
        "{}",
        broken.describe()
    );
    // A tenth of that is inside the noise floor of a real GPU.
    let tiny = GradeConfig {
        gain: [1.002; 3],
        ..GradeConfig::default()
    };
    let quiet = verdict(&golden, &graded_scene(&tiny));
    assert!(
        quiet.matches(),
        "a 0.002 gain nudge must pass: {}",
        quiet.describe()
    );
    assert!(quiet.mean_abs_milli < 500, "{}", quiet.describe());
}

#[test]
fn the_gate_fails_when_a_patch_goes_wrong_and_passes_a_speck() {
    let golden = graded_scene(&GradeConfig::default());
    // A 4x20 patch (2.6% of the frame) loses 30 levels of red: a lost
    // highlight. The mean barely moves; the far-off share breaks.
    let mut patch = golden.clone();
    for row in 0..20usize {
        for col in 0..4usize {
            let at = (row * 64 + col + 30) * 4;
            patch[at] = patch[at].saturating_sub(30);
        }
    }
    let wrong = verdict(&golden, &patch);
    assert!(
        wrong.over_ppm > GOLDEN_OVER_PPM && wrong.mean_ok && !wrong.matches(),
        "{}",
        wrong.describe()
    );
    // A speck of 3 pixels (0.1% of the frame) with the same error passes.
    let mut speck = golden.clone();
    for col in 0..3usize {
        let at = (col + 30) * 4;
        speck[at] = speck[at].saturating_sub(30);
    }
    let small = verdict(&golden, &speck);
    assert!(small.matches(), "{}", small.describe());
    assert_eq!(small.over_pixels, 3, "the speck really is far off");
}
