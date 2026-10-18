//! post_fsr1_parity: pin the FSR1-shape EASU constants.
//!
//! Purpose: `post_aa_parity` pins the RCAS sharpen only; this file pins the
//! EASU side so that a one-constant edit turns it red. Held here: the
//! dynamic-resolution constants `fsr_easu_con` hands the upscaler (scale,
//! offset, texel size, as in FSR1 `FsrEasuCon`), the sample position law
//! those constants feed (a ramp lands on the pixel-centre law), the flat-field
//! identity and the 2x2 ringing clamp. Owner is the render lane; consumer is
//! the spatial upscale beside the frozen bilinear TAAU. Units: pixels for
//! sizes. Determinism: same inputs give the same pixels. Errors: bad sizes
//! fail loudly via `PostError`.
//!
//! Attribution (idea-only, zero donor bytes): the constant shape follows the
//! *idea* of FidelityFX FSR1 `FsrEasuCon` (MIT). Every line below calls public
//! items only and asserts fresh vectors against them.

use engine_render::post::{fsr_easu_con, fsr_easu_upsample};

const LOW: f32 = 0.1;
const HIGH: f32 = 0.9;
const EPS: f32 = 1e-5;
/// Lanczos-2 ripple on an integer ramp stays under this (measured 0.054).
const RAMP_TOLERANCE: f32 = 0.08;

/// A gray `w` by `h` source whose value at `(x, y)` is `f(x, y)`.
fn source(w: u32, h: u32, f: impl Fn(u32, u32) -> f32) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let v = f(x, y);
            out.push([v, v, v]);
        }
    }
    out
}

/// Gray value of texel `(x, y)` in a frame `w` wide.
fn at(frame: &[[f32; 3]], w: u32, x: u32, y: u32) -> f32 {
    frame[usize::try_from(y * w + x).expect("texel index fits usize")][0]
}

/// Lossless `u32` to `f32` for the small test coordinates.
fn f32_from_small(v: u32) -> f32 {
    f32::from(u16::try_from(v).expect("test coordinate fits u16"))
}

#[test]
fn easu_con_pins_scale_offset_and_texel() {
    // Exact in binary: 960x540 to 1920x1080 is a clean 2x.
    let half = fsr_easu_con(960, 540, 1920, 1080).expect("2x builds");
    assert_eq!(half.scale, [0.5, 0.5], "scale is src over dst");
    assert_eq!(half.offset, [-0.25, -0.25], "offset is 0.5 * scale - 0.5");
    assert_eq!(
        half.texel,
        [1.0 / 960.0, 1.0 / 540.0],
        "texel is 1 over src"
    );
    assert_eq!(
        (half.src_w, half.src_h, half.dst_w, half.dst_h),
        (960, 540, 1920, 1080)
    );
    // A 2/3 scale (720p to 1080p) in f32, same expressions as the builder.
    let fc3 = fsr_easu_con(1280, 720, 1920, 1080).expect("720p to 1080p builds");
    let (sx, sy) = (1280.0_f32 / 1920.0_f32, 720.0_f32 / 1080.0_f32);
    assert_eq!(fc3.scale, [sx, sy]);
    assert_eq!(fc3.offset, [0.5 * sx - 0.5, 0.5 * sy - 0.5]);
    assert_eq!(fc3.texel, [1.0 / 1280.0, 1.0 / 720.0]);
    assert!(fsr_easu_con(0, 4, 8, 8).is_err(), "zero size refused");
}

#[test]
fn ramp_follows_the_sample_law_the_constants_feed() {
    // Output texel x samples the source at (x + 0.5) * scale + offset, with
    // scale and offset straight from `fsr_easu_con`. A horizontal ramp v = x
    // returns that coordinate, so an edit to the constants or to the taps
    // moves it. Open finding: FSR1 feeds the offset the integer pixel
    // position; this adds 0.5 first, so it sits 0.5 * scale (0.25 texel at 2x)
    // right of the TAAU centre law (x + 0.5) * scale - 0.5. Fixing that
    // drops the 540p MTF50 lift gate in `post.rs` to 0, so it needs a decision.
    let con = fsr_easu_con(16, 16, 32, 32).expect("2x builds");
    let ramp = source(16, 16, |x, _| f32_from_small(x));
    let frame = fsr_easu_upsample(&ramp, 16, 16, 32, 32).expect("ramp upscales");
    for x in 3..29 {
        let law = (f32_from_small(x) + 0.5) * con.scale[0] + con.offset[0];
        let got = at(&frame, 32, x, 16);
        assert!(
            (got - law).abs() <= RAMP_TOLERANCE,
            "texel {x}: got {got}, sample law {law}"
        );
    }
}

#[test]
fn flat_field_is_bit_exact() {
    let flat = source(8, 8, |_, _| 0.4);
    let frame = fsr_easu_upsample(&flat, 8, 8, 16, 16).expect("flat upscales");
    assert!(
        frame.iter().all(|p| *p == [0.4, 0.4, 0.4]),
        "a flat field survives untouched"
    );
}

#[test]
fn step_edge_never_rings_past_its_two_sides() {
    // Lanczos-2 undershoots and overshoots a hard step; the 2x2 min/max clamp
    // must hold every texel inside [LOW, HIGH], and the edge still blends.
    let step = source(16, 16, |x, _| if x < 8 { LOW } else { HIGH });
    let frame = fsr_easu_upsample(&step, 16, 16, 32, 32).expect("step upscales");
    let mut between = 0;
    for x in 0..32 {
        let v = at(&frame, 32, x, 16);
        assert!(
            (LOW - EPS..=HIGH + EPS).contains(&v),
            "texel {x} rings past the edge: {v}"
        );
        if v > LOW + 1e-3 && v < HIGH - 1e-3 {
            between += 1;
        }
    }
    assert!(between > 0, "the edge blends at least one texel");
}
