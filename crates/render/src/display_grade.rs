//! display_grade: the CPU twin of the two display options `shaders/tonemap.wgsl`
//! carries beside the hue-keeping ACES fit: the Khronos PBR Neutral curve
//! (`override DISPLAY_NEUTRAL`, `pbr_neutral`) and the radial grade vignette
//! (`grade_vignette`, named strength and radius). The WGSL chunk is the source;
//! the constants here are checked against its text by the tests below.

/// Grade vignette strength: how far the corners fall, `0` is off. The WGSL
/// `GRADE_VIGNETTE_STRENGTH` twin.
pub const GRADE_VIGNETTE_STRENGTH: f32 = 0.20;
/// Grade vignette radius: the fraction of the centre-to-corner distance inside
/// which the frame is untouched, `0..1`. The WGSL `GRADE_VIGNETTE_RADIUS` twin.
pub const GRADE_VIGNETTE_RADIUS: f32 = 0.60;

/// Khronos PBR Neutral shoulder start and highlight desaturation, the WGSL
/// `NEUTRAL_START` and `NEUTRAL_DESAT` twins.
const NEUTRAL_START: f32 = 0.76;
const NEUTRAL_DESAT: f32 = 0.15;

/// The vignette factor at one uv (`0..=1` on both axes): `1` inside the radius,
/// `1 - strength` at a corner, smooth in between. Units: a multiplier on the
/// display value. Pure and finite for finite input.
pub fn grade_vignette_factor(uv: [f32; 2], strength: f32, radius: f32) -> f32 {
    if strength <= 0.0 {
        return 1.0;
    }
    let dx = uv[0] - 0.5;
    let dy = uv[1] - 0.5;
    let r2 = (dx * dx + dy * dy) * 2.0;
    let edge = radius * radius;
    let t = ((r2 - edge) / (1.0 - edge)).clamp(0.0, 1.0);
    1.0 - strength * (t * t * (3.0 - 2.0 * t))
}

/// Applies the grade vignette to one display colour: the three channels share
/// one factor, so the hue is kept.
pub fn grade_vignette(color: [f32; 3], uv: [f32; 2], strength: f32, radius: f32) -> [f32; 3] {
    let k = grade_vignette_factor(uv, strength, radius);
    [color[0] * k, color[1] * k, color[2] * k]
}

/// Khronos PBR Neutral: the subtractive black offset, then the highlight
/// shoulder. Linear input, display `0..=1` out, the WGSL `pbr_neutral` twin.
/// Below the shoulder the only change is one offset shared by all channels, so
/// every channel difference (the hue) survives.
pub fn pbr_neutral(color: [f32; 3]) -> [f32; 3] {
    let c = color.map(|v| v.max(0.0));
    let x = c[0].min(c[1]).min(c[2]);
    let offset = if x < 0.08 { x - 6.25 * x * x } else { 0.04 };
    let mut out = c.map(|v| v - offset);
    let peak = out[0].max(out[1]).max(out[2]);
    if peak < NEUTRAL_START {
        return out;
    }
    let d = 1.0 - NEUTRAL_START;
    let new_peak = 1.0 - d * d / (peak + d - NEUTRAL_START);
    let k = new_peak / peak;
    out = out.map(|v| v * k);
    let g = 1.0 - 1.0 / (NEUTRAL_DESAT * (peak - new_peak) + 1.0);
    out.map(|v| v + (new_peak - v) * g)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WGSL: &str = include_str!("shaders/tonemap.wgsl");

    #[test]
    fn vignette_darkens_corners_not_center() {
        let s = GRADE_VIGNETTE_STRENGTH;
        let r = GRADE_VIGNETTE_RADIUS;
        // The centre is untouched, and so is anything inside the radius.
        assert_eq!(grade_vignette_factor([0.5, 0.5], s, r), 1.0);
        assert_eq!(grade_vignette_factor([0.6, 0.5], s, r), 1.0);
        // Each corner is darker by exactly the named strength.
        for uv in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]] {
            let k = grade_vignette_factor(uv, s, r);
            assert!((k - (1.0 - s)).abs() < 1e-6, "corner {uv:?} factor {k}");
        }
        // Falloff is monotone from the centre out to the corner.
        let edge = grade_vignette_factor([0.0, 0.5], s, r);
        let corner = grade_vignette_factor([0.0, 0.0], s, r);
        assert!(corner < edge && edge < 1.0, "edge {edge}, corner {corner}");
        // The colour darkens the same way and keeps its hue (equal ratios).
        let c = grade_vignette([0.8, 0.4, 0.2], [0.0, 0.0], s, r);
        assert!((c[0] / c[2] - 4.0).abs() < 1e-5, "hue ratio {c:?}");
        assert!(c[0] < 0.8 && c[2] < 0.2, "corner colour {c:?}");
    }

    #[test]
    fn vignette_strength_zero_is_identity() {
        let colour = [0.9f32, 0.31, 0.05];
        for uv in [[0.0, 0.0], [0.25, 0.7], [0.5, 0.5], [1.0, 1.0]] {
            assert_eq!(grade_vignette_factor(uv, 0.0, GRADE_VIGNETTE_RADIUS), 1.0);
            assert_eq!(grade_vignette(colour, uv, 0.0, GRADE_VIGNETTE_RADIUS), colour);
        }
    }

    #[test]
    fn vignette_constants_match_the_wgsl_chunk() {
        assert!(WGSL.contains(&format!(
            "const GRADE_VIGNETTE_STRENGTH: f32 = {GRADE_VIGNETTE_STRENGTH:.2};"
        )));
        assert!(WGSL.contains(&format!(
            "const GRADE_VIGNETTE_RADIUS: f32 = {GRADE_VIGNETTE_RADIUS:.2};"
        )));
        assert!(WGSL.contains("fn grade_vignette(c: vec3<f32>, uv: vec2<f32>)"));
        assert!(WGSL.contains("override DISPLAY_NEUTRAL: bool = false;"));
        assert!(WGSL.contains(&format!("const NEUTRAL_START: f32 = {NEUTRAL_START:.2};")));
    }

    #[test]
    fn tonemap_pbr_neutral_keeps_the_hue_below_the_shoulder() {
        // Mid and dark values: one shared offset, so channel differences hold.
        let input = [0.5f32, 0.2, 0.05];
        let out = pbr_neutral(input);
        assert!(((out[0] - out[1]) - (input[0] - input[1])).abs() < 1e-6, "{out:?}");
        assert!(((out[1] - out[2]) - (input[1] - input[2])).abs() < 1e-6, "{out:?}");
        // Black stays black, and nothing goes negative.
        assert_eq!(pbr_neutral([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        assert!(pbr_neutral([0.0, 0.01, 0.02]).iter().all(|v| *v >= 0.0));
    }

    #[test]
    fn tonemap_pbr_neutral_is_bounded_and_monotone_in_the_shoulder() {
        let mut last = 0.0f32;
        for step in 0..=200 {
            let v = step as f32 * 0.05;
            let out = pbr_neutral([v, v, v]);
            assert!(out.iter().all(|x| (0.0..=1.0).contains(x)), "{v}: {out:?}");
            assert!(out[0] + 1e-6 >= last, "not monotone at {v}");
            last = out[0];
        }
        let huge = pbr_neutral([1e6, 1.0, 0.5]);
        assert!(huge.iter().all(|x| (0.0..=1.0).contains(x)), "{huge:?}");
    }
}
