// Donor: arXiv:2006.13846 (paper, no code), IDEA-ONLY.
// Pattern only: 11x11 Gaussian-window SSIM with flat-window guards plus a
// per-region floor; fresh pure-Rust write beside capture_compare.rs,
// no donor code copied.

//! Windowed SSIM scoring for captures as small grayscale frames.
//!
//! Each `11x11` window scores the standard luminance-contrast-structure
//! product with stabilizers `C1`/`C2`; flat windows take an explicit guard
//! so they never yield `NaN`, and callers may pass a per-region floor that
//! clamps each window before the mean.

/// Gaussian window side length.
pub const WINDOW_SIDE: usize = 11;
/// Gaussian sigma for the `11x11` window.
pub const WINDOW_SIGMA: f64 = 1.5;
/// Stabilizer for the luminance term (`(0.01 * 255)^2`).
pub const SSIM_C1: f64 = 6.5025;
/// Stabilizer for the contrast term (`(0.03 * 255)^2`).
pub const SSIM_C2: f64 = 58.5225;

/// `11x11` Gaussian kernel with unit sum, sigma [`WINDOW_SIGMA`].
pub fn gaussian_kernel_11() -> [[f64; WINDOW_SIDE]; WINDOW_SIDE] {
    let mut kernel = [[0.0_f64; WINDOW_SIDE]; WINDOW_SIDE];
    let mut sum = 0.0_f64;
    let half: f64 = 5.0;
    let denom: f64 = 2.0 * WINDOW_SIGMA * WINDOW_SIGMA;
    for (y, row) in kernel.iter_mut().enumerate() {
        for (x, cell) in row.iter_mut().enumerate() {
            let dx: f64 = x as f64 - half;
            let dy: f64 = y as f64 - half;
            let value: f64 = (-(dx * dx + dy * dy) / denom).exp();
            *cell = value;
            sum += value;
        }
    }
    for row in kernel.iter_mut() {
        for weight in row.iter_mut() {
            *weight /= sum;
        }
    }
    kernel
}

/// SSIM of one weighted patch pair.
///
/// `a`, `b` and `weights` share one length; `weights` should sum to `1.0`.
/// Flat pairs (near-zero variance on both sides) return `1.0` when the means
/// match and the luminance-only term otherwise, so the result is never `NaN`.
/// Output is clamped to `0.0..=1.0`.
pub fn window_ssim(a: &[u8], b: &[u8], weights: &[f64]) -> f64 {
    assert_eq!(a.len(), b.len(), "patch lengths must match");
    assert_eq!(a.len(), weights.len(), "weights must match patch");
    assert!(!a.is_empty(), "patch must not be empty");
    let mut mu_x = 0.0_f64;
    let mut mu_y = 0.0_f64;
    for i in 0..a.len() {
        let w = weights[i];
        mu_x += w * f64::from(a[i]);
        mu_y += w * f64::from(b[i]);
    }
    let mut var_x = 0.0_f64;
    let mut var_y = 0.0_f64;
    let mut covar = 0.0_f64;
    for i in 0..a.len() {
        let w = weights[i];
        let dx = f64::from(a[i]) - mu_x;
        let dy = f64::from(b[i]) - mu_y;
        var_x += w * dx * dx;
        var_y += w * dy * dy;
        covar += w * dx * dy;
    }
    // Flat-window guard: both variances near zero means each side is a solid
    // shade. Identical shades score 1.0; differing shades fall back to the
    // luminance term, which stays well-defined because C1 > 0.
    if var_x + var_y < 1e-12 {
        if (mu_x - mu_y).abs() < 1e-6 {
            return 1.0;
        }
        let luminance = (2.0 * mu_x * mu_y + SSIM_C1) / (mu_x * mu_x + mu_y * mu_y + SSIM_C1);
        return luminance.clamp(0.0, 1.0);
    }
    let numerator = (2.0 * mu_x * mu_y + SSIM_C1) * (2.0 * covar + SSIM_C2);
    let denominator = (mu_x * mu_x + mu_y * mu_y + SSIM_C1) * (var_x + var_y + SSIM_C2);
    if denominator <= 0.0 || !denominator.is_finite() {
        return 1.0;
    }
    let score = numerator / denominator;
    if !score.is_finite() {
        return 1.0;
    }
    score.clamp(0.0, 1.0)
}

/// Mean windowed SSIM of two `width` by `height` grayscale frames.
///
/// Slides the `11x11` Gaussian window over every fully-contained position;
/// frames smaller than the window score as one uniform window. Returns `1.0`
/// when both frames are empty and `0.0` when lengths differ or the geometry
/// does not match the buffers.
pub fn ssim_score(a: &[u8], b: &[u8], width: u32, height: u32) -> f64 {
    ssim_score_with_floor(a, b, width, height, 0.0)
}

/// Mean windowed SSIM with a per-region floor.
///
/// Each window scores `max(window, floor)` before the mean, so one dark
/// patch cannot drag the whole frame below `floor`. `floor` is clamped to
/// `0.0..=1.0`.
pub fn ssim_score_with_floor(a: &[u8], b: &[u8], width: u32, height: u32, floor: f64) -> f64 {
    let floor = floor.clamp(0.0, 1.0);
    let (w, h) = (width as usize, height as usize);
    if a.len() != b.len() {
        return 0.0;
    }
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || w == 0 || h == 0 {
        return 0.0;
    }
    if a.len() != w * h {
        return 0.0;
    }
    // Small frames never fit the Gaussian window: score one uniform window.
    if w < WINDOW_SIDE || h < WINDOW_SIDE {
        let weight = 1.0 / a.len() as f64;
        let weights = vec![weight; a.len()];
        return window_ssim(a, b, &weights).max(floor);
    }
    let kernel = gaussian_kernel_11();
    let mut flat_weights = Vec::with_capacity(WINDOW_SIDE * WINDOW_SIDE);
    for row in kernel.iter() {
        for weight in row.iter() {
            flat_weights.push(*weight);
        }
    }
    let mut patch_a = vec![0_u8; WINDOW_SIDE * WINDOW_SIDE];
    let mut patch_b = vec![0_u8; WINDOW_SIDE * WINDOW_SIDE];
    let mut sum = 0.0_f64;
    let mut count = 0_usize;
    for top in 0..=(h - WINDOW_SIDE) {
        for left in 0..=(w - WINDOW_SIDE) {
            for y in 0..WINDOW_SIDE {
                for x in 0..WINDOW_SIDE {
                    let at = (top + y) * w + (left + x);
                    patch_a[y * WINDOW_SIDE + x] = a[at];
                    patch_b[y * WINDOW_SIDE + x] = b[at];
                }
            }
            let window = window_ssim(&patch_a, &patch_b, &flat_weights);
            sum += window.max(floor);
            count += 1;
        }
    }
    if count == 0 {
        return 0.0;
    }
    (sum / count as f64).clamp(0.0, 1.0)
}

/// Assert `ssim_score` meets `threshold`, printing the score on failure.
pub fn assert_ssim(a: &[u8], b: &[u8], width: u32, height: u32, threshold: f64) {
    let score = ssim_score(a, b, width, height);
    assert!(
        score >= threshold,
        "capture SSIM {score:.6} below threshold {threshold:.6} for {width}x{height}"
    );
}

fn gradient_frame(width: usize, height: usize) -> Vec<u8> {
    let mut frame = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let value = ((x + y * width) % 256) as u8;
            frame.push(value);
        }
    }
    frame
}

#[test]
fn identical_frames_score_one() {
    let (width, height) = (16_u32, 16_u32);
    let golden = gradient_frame(width as usize, height as usize);
    let current = golden.clone();
    let score = ssim_score(&current, &golden, width, height);
    println!("passed identical SSIM {score:.6}");
    assert!(
        (score - 1.0).abs() < 1e-9,
        "identical frames must score 1.0, got {score}"
    );
    assert_ssim(&current, &golden, width, height, 1.0);
    // Identical flat frame also scores 1.0 through the flat guard.
    let flat = vec![128_u8; (width as usize) * (height as usize)];
    let flat_score = ssim_score(&flat, &flat, width, height);
    println!("passed identical flat SSIM {flat_score:.6}");
    assert!(
        (flat_score - 1.0).abs() < 1e-9,
        "identical flat must score 1.0, got {flat_score}"
    );
}

#[test]
fn flat_windows_never_nan() {
    let (width, height) = (16_u32, 16_u32);
    let len = (width as usize) * (height as usize);
    let black = vec![0_u8; len];
    let white = vec![255_u8; len];
    let gray = vec![128_u8; len];
    let gray_shift = vec![129_u8; len];
    let pairs: [(&Vec<u8>, &Vec<u8>, &str); 5] = [
        (&black, &black, "black-black"),
        (&white, &white, "white-white"),
        (&gray, &gray, "gray-gray"),
        (&black, &white, "black-white"),
        (&gray, &gray_shift, "gray-shift"),
    ];
    for (a, b, name) in pairs {
        let score = ssim_score(a, b, width, height);
        let floored = ssim_score_with_floor(a, b, width, height, 0.25);
        println!("passed flat {name} SSIM {score:.6} floored {floored:.6}");
        assert!(!score.is_nan(), "{name} score is NaN");
        assert!(!floored.is_nan(), "{name} floored score is NaN");
        assert!(
            (0.0..=1.0).contains(&score),
            "{name} score {score} outside 0..=1"
        );
        assert!(
            (0.0..=1.0).contains(&floored),
            "{name} floored {floored} outside 0..=1"
        );
    }
    // Identical flats must read exactly 1.0; opposing flats stay below it.
    assert!((ssim_score(&black, &black, width, height) - 1.0).abs() < 1e-9);
    assert!((ssim_score(&gray, &gray, width, height) - 1.0).abs() < 1e-9);
    assert!(ssim_score(&black, &white, width, height) < 1.0);
    // Frames smaller than the window use the uniform fallback, still no NaN.
    let tiny_black = vec![0_u8; 4 * 4];
    let tiny_white = vec![255_u8; 4 * 4];
    let tiny_same = ssim_score(&tiny_black, &tiny_black, 4, 4);
    let tiny_diff = ssim_score(&tiny_black, &tiny_white, 4, 4);
    println!("passed tiny flat same {tiny_same:.6} diff {tiny_diff:.6}");
    assert!(!tiny_same.is_nan() && !tiny_diff.is_nan());
    assert!((tiny_same - 1.0).abs() < 1e-9);
    assert!((0.0..=1.0).contains(&tiny_diff));
}

#[test]
fn single_pixel_drop_and_per_region_floor() {
    let (width, height) = (16_u32, 16_u32);
    let golden = gradient_frame(width as usize, height as usize);
    let mut current = golden.clone();
    current[100] = current[100].wrapping_add(255);
    let score = ssim_score(&current, &golden, width, height);
    println!("passed one-pixel SSIM {score:.6}");
    assert!(!score.is_nan(), "score is NaN");
    assert!(
        score > 0.0 && score < 1.0,
        "expected score in (0, 1), got {score}"
    );
    let floor = 0.9;
    let floored = ssim_score_with_floor(&current, &golden, width, height, floor);
    println!("passed floored SSIM {floored:.6} at floor {floor:.2}");
    assert!(
        floored >= score,
        "floored {floored:.6} must not sit below raw {score:.6}"
    );
    assert!(
        floored >= floor,
        "floored {floored:.6} must respect per-region floor {floor:.2}"
    );
    // A wholly opposed frame still respects its floor.
    let black = vec![0_u8; golden.len()];
    let white = vec![255_u8; golden.len()];
    let opposed = ssim_score_with_floor(&black, &white, width, height, floor);
    println!("passed opposed floored SSIM {opposed:.6}");
    assert!((opposed - floor).abs() < 1e-9, "opposed got {opposed}");
    let threshold = 1.0;
    let result = std::panic::catch_unwind(|| {
        assert_ssim(&current, &golden, width, height, threshold);
    });
    assert!(result.is_err(), "expected threshold {threshold} to fail");
}
