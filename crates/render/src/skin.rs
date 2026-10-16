//! skin, eye and hair shade slice: wrapped-diffuse subsurface approx plus
//! wet-eye and strand glint, gated by the quality table.
//!
//! Owner: render lane. Consumer: the forward-pass bind, next slice, which
//! calls [`shade_skin`], [`shade_eye`] and [`shade_hair`] per fighter pixel
//! and compiles [`SKIN_WGSL`] beside the forward shader. Units: directions
//! are triplets in meters (normalized inside), colors are linear, wrap,
//! thickness and shift are unitless. Determinism: fixed scalar order with
//! `+ - * /`, `sqrt` and integer powers only; same inputs give bit-identical
//! outputs on every platform. Errors are typed [`ShadeError`]; nothing here
//! enters any simulation hash and nothing allocates per call.

use std::fmt;

use crate::quality::Quality;

// ---------------------------------------------------------------------------
// Slice source shared with the GPU bind.
// ---------------------------------------------------------------------------

/// Slice WGSL source for the forward-pass bind, next slice. Linear light in,
/// linear light out; the blit tone-maps once, never here. The forward BRDF
/// stays in its own file; this text only adds the slice terms.
pub const SKIN_WGSL: &str = include_str!("shaders/skin.wgsl");

// ---------------------------------------------------------------------------
// Quality gate: which slice terms run per tier.
// ---------------------------------------------------------------------------

/// Slice terms enabled for one frame. Both start off; higher tiers switch
/// terms on. Plain data, no clock, no float.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SkinLevel {
    /// Wrapped transmission bleed on skin.
    pub subsurface: bool,
    /// Wet-eye and strand glint highlights.
    pub glint: bool,
}

/// Terms for a quality tier: the lowest tier runs bare wrapped diffuse, the
/// middle tier adds subsurface transmission, the highest tier adds glint
/// too. Pure match, identical on every platform.
#[must_use]
pub fn skin_level(quality: Quality) -> SkinLevel {
    match quality {
        Quality::Potato => SkinLevel {
            subsurface: false,
            glint: false,
        },
        Quality::Comfort => SkinLevel {
            subsurface: true,
            glint: false,
        },
        Quality::High => SkinLevel {
            subsurface: true,
            glint: true,
        },
    }
}

// ---------------------------------------------------------------------------
// Errors: typed, never a string, never silent.
// ---------------------------------------------------------------------------

/// Shade evaluation failures; every bad input maps to one variant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShadeError {
    /// A float input is NaN or infinite; lane carried.
    NonFinite(&'static str),
    /// A direction has no usable length; lane carried.
    Degenerate(&'static str),
    /// A param sits outside its documented range; lane carried.
    OutOfRange(&'static str),
}

impl fmt::Display for ShadeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(lane) => write!(f, "shade {lane} is not finite"),
            Self::Degenerate(lane) => write!(f, "shade {lane} has no direction"),
            Self::OutOfRange(lane) => write!(f, "shade {lane} is out of range"),
        }
    }
}

impl std::error::Error for ShadeError {}

// ---------------------------------------------------------------------------
// Scalar core: one owner per shape, shared by the three shade terms.
// ---------------------------------------------------------------------------

/// Dot product of two triplets.
fn dot3(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Sum of two triplets.
fn add3(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// Finite check for one lane.
fn finite(lane: &'static str, v: f32) -> Result<(), ShadeError> {
    if !v.is_finite() {
        return Err(ShadeError::NonFinite(lane));
    }
    Ok(())
}

/// Unit-range check for one lane.
fn unit(lane: &'static str, v: f32) -> Result<(), ShadeError> {
    finite(lane, v)?;
    if v < 0.0 {
        return Err(ShadeError::OutOfRange(lane));
    }
    if v > 1.0 {
        return Err(ShadeError::OutOfRange(lane));
    }
    Ok(())
}

/// Linear color check for one lane: finite and non-negative per channel.
fn rgb(lane: &'static str, c: &[f32; 3]) -> Result<(), ShadeError> {
    for v in c.iter() {
        finite(lane, *v)?;
        if *v < 0.0 {
            return Err(ShadeError::OutOfRange(lane));
        }
    }
    Ok(())
}

/// Unit-length copy of a direction. Rejects non-finite lanes and tiny
/// vectors before any divide, so no NaN ever flows into the shade terms.
fn normalize3(v: &[f32; 3], lane: &'static str) -> Result<[f32; 3], ShadeError> {
    for c in v.iter() {
        finite(lane, *c)?;
    }
    let len_sq = dot3(v, v);
    if len_sq < 1e-12 {
        return Err(ShadeError::Degenerate(lane));
    }
    let len = len_sq.sqrt();
    Ok([v[0] / len, v[1] / len, v[2] / len])
}

// ---------------------------------------------------------------------------
// Params: wrapped skin, wet eye, strand hair.
// ---------------------------------------------------------------------------

/// Skin shape: wrap amount, scatter tint and tissue thickness.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SkinParams {
    /// Wrap amount, `0..=1` (unitless). Zero is bare clamped diffuse.
    pub wrap: f32,
    /// Scatter tint in linear space, finite and non-negative.
    pub scatter: [f32; 3],
    /// Tissue thickness, `0..=1` (unitless). Zero disables bleed.
    pub thickness: f32,
}

impl SkinParams {
    /// Validate ranges before any shade math.
    pub fn validate(&self) -> Result<(), ShadeError> {
        unit("skin.wrap", self.wrap)?;
        rgb("skin.scatter", &self.scatter)?;
        unit("skin.thickness", self.thickness)?;
        Ok(())
    }
}

/// Wet-eye shape: cornea glint strength and tightness.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct EyeParams {
    /// Glint strength, `0..=1` (linear units added to diffuse).
    pub glint_strength: f32,
    /// Glint tightness, `1..=256` (integer power on the half-vector dot).
    pub glint_sharp: i32,
}

impl EyeParams {
    /// Validate ranges before any shade math.
    pub fn validate(&self) -> Result<(), ShadeError> {
        unit("eye.glint_strength", self.glint_strength)?;
        if self.glint_sharp < 1 {
            return Err(ShadeError::OutOfRange("eye.glint_sharp"));
        }
        if self.glint_sharp > 256 {
            return Err(ShadeError::OutOfRange("eye.glint_sharp"));
        }
        Ok(())
    }
}

/// Strand-hair shape: glint tint, band shift and strength.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HairParams {
    /// Glint tint in linear space, finite and non-negative.
    pub tint: [f32; 3],
    /// Band shift along the strand, `-1..=1` (unitless).
    pub shift: f32,
    /// Glint strength, `0..=1` (linear units added to diffuse).
    pub glint_strength: f32,
    /// Glint tightness, `1..=256` (integer power on the band).
    pub glint_sharp: i32,
}

impl HairParams {
    /// Validate ranges before any shade math.
    pub fn validate(&self) -> Result<(), ShadeError> {
        rgb("hair.tint", &self.tint)?;
        finite("hair.shift", self.shift)?;
        if self.shift < -1.0 {
            return Err(ShadeError::OutOfRange("hair.shift"));
        }
        if self.shift > 1.0 {
            return Err(ShadeError::OutOfRange("hair.shift"));
        }
        unit("hair.glint_strength", self.glint_strength)?;
        if self.glint_sharp < 1 {
            return Err(ShadeError::OutOfRange("hair.glint_sharp"));
        }
        if self.glint_sharp > 256 {
            return Err(ShadeError::OutOfRange("hair.glint_sharp"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Shade terms: skin, eye, hair under one light.
// ---------------------------------------------------------------------------

/// Skin under one light: wrapped diffuse plus gated transmission bleed.
/// `albedo` is linear; the directions need not be unit length (normalized
/// inside). Returns linear radiance before exposure. Disabled transmission
/// returns bare wrapped diffuse, so the tier gate always changes the
/// backlit picture. Errors on bad inputs, never NaN.
pub fn shade_skin(
    albedo: &[f32; 3],
    params: &SkinParams,
    level: &SkinLevel,
    normal: &[f32; 3],
    light_dir: &[f32; 3],
    view_dir: &[f32; 3],
) -> Result<[f32; 3], ShadeError> {
    rgb("skin.albedo", albedo)?;
    params.validate()?;
    let n = normalize3(normal, "skin.normal")?;
    let l = normalize3(light_dir, "skin.light")?;
    let v = normalize3(view_dir, "skin.view")?;
    let ndotl = dot3(&n, &l).clamp(-1.0, 1.0);
    let wrapped = ((ndotl + params.wrap) / (1.0 + params.wrap)).clamp(0.0, 1.0);
    let mut out = [
        albedo[0] * wrapped,
        albedo[1] * wrapped,
        albedo[2] * wrapped,
    ];
    if level.subsurface {
        let through = (-dot3(&v, &l) * 0.5 + 0.5).clamp(0.0, 1.0);
        let bleed = through.powi(3) * params.thickness;
        out[0] += params.scatter[0] * bleed;
        out[1] += params.scatter[1] * bleed;
        out[2] += params.scatter[2] * bleed;
    }
    Ok(out)
}

/// Wet eye under one light: soft diffuse plus a gated cornea glint. A
/// collapsed half-vector (light opposite view) keeps diffuse and skips the
/// glint instead of dividing by zero. Errors on bad inputs, never NaN.
pub fn shade_eye(
    albedo: &[f32; 3],
    params: &EyeParams,
    level: &SkinLevel,
    normal: &[f32; 3],
    light_dir: &[f32; 3],
    view_dir: &[f32; 3],
) -> Result<[f32; 3], ShadeError> {
    rgb("eye.albedo", albedo)?;
    params.validate()?;
    let n = normalize3(normal, "eye.normal")?;
    let l = normalize3(light_dir, "eye.light")?;
    let v = normalize3(view_dir, "eye.view")?;
    let ndotl = dot3(&n, &l).clamp(0.0, 1.0);
    let mut out = [albedo[0] * ndotl, albedo[1] * ndotl, albedo[2] * ndotl];
    if level.glint {
        let h = add3(&l, &v);
        let h_len_sq = dot3(&h, &h);
        if h_len_sq >= 1e-12 {
            let len = h_len_sq.sqrt();
            let hn = [h[0] / len, h[1] / len, h[2] / len];
            let ndoth = dot3(&n, &hn).clamp(0.0, 1.0);
            let glint = params.glint_strength * ndoth.powi(params.glint_sharp);
            out[0] += glint;
            out[1] += glint;
            out[2] += glint;
        }
    }
    Ok(out)
}

/// Hair strand under one light: soft diffuse plus a gated anisotropic glint
/// band around the tangent-perpendicular plane. `tangent` runs along the
/// strand; `shift` slides the band. Errors on bad inputs, never NaN.
pub fn shade_hair(
    albedo: &[f32; 3],
    params: &HairParams,
    level: &SkinLevel,
    normal: &[f32; 3],
    tangent: &[f32; 3],
    light_dir: &[f32; 3],
    view_dir: &[f32; 3],
) -> Result<[f32; 3], ShadeError> {
    rgb("hair.albedo", albedo)?;
    params.validate()?;
    let n = normalize3(normal, "hair.normal")?;
    let t = normalize3(tangent, "hair.tangent")?;
    let l = normalize3(light_dir, "hair.light")?;
    let v = normalize3(view_dir, "hair.view")?;
    let ndotl = dot3(&n, &l).clamp(0.0, 1.0);
    let mut out = [albedo[0] * ndotl, albedo[1] * ndotl, albedo[2] * ndotl];
    if level.glint {
        let h = add3(&l, &v);
        let h_len_sq = dot3(&h, &h);
        if h_len_sq >= 1e-12 {
            let len = h_len_sq.sqrt();
            let hn = [h[0] / len, h[1] / len, h[2] / len];
            let th = (dot3(&t, &hn) - params.shift).clamp(-1.0, 1.0);
            let band = (1.0 - th * th).clamp(0.0, 1.0).sqrt();
            let glint = params.glint_strength * band.powi(params.glint_sharp);
            out[0] += params.tint[0] * glint;
            out[1] += params.tint[1] * glint;
            out[2] += params.tint[2] * glint;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-computed oracle tolerance in linear units.
    const TOL: f32 = 1e-5;

    fn skin_params() -> SkinParams {
        SkinParams {
            wrap: 0.5,
            scatter: [0.9, 0.4, 0.3],
            thickness: 0.6,
        }
    }

    fn eye_params() -> EyeParams {
        EyeParams {
            glint_strength: 0.8,
            glint_sharp: 64,
        }
    }

    fn hair_params() -> HairParams {
        HairParams {
            tint: [0.8, 0.7, 0.6],
            shift: 0.0,
            glint_strength: 0.6,
            glint_sharp: 32,
        }
    }

    fn approx(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (*x - *y).abs() <= TOL)
    }

    #[test]
    fn tiers_gate_terms_in_order() {
        assert_eq!(
            skin_level(Quality::Potato),
            SkinLevel {
                subsurface: false,
                glint: false,
            }
        );
        assert_eq!(
            skin_level(Quality::Comfort),
            SkinLevel {
                subsurface: true,
                glint: false,
            }
        );
        assert_eq!(
            skin_level(Quality::High),
            SkinLevel {
                subsurface: true,
                glint: true,
            }
        );
    }

    #[test]
    fn subsurface_lifts_backlit_skin() {
        // Face normal and viewer forward, light behind the head: wrapped
        // diffuse is zero, transmission is full, so the gate carries the
        // whole backlit picture: scatter * thickness = [0.54, 0.24, 0.18].
        let albedo = [0.55, 0.35, 0.28];
        let normal = [0.0, 0.0, 1.0];
        let light = [0.0, 0.0, -1.0];
        let view = [0.0, 0.0, 1.0];
        let off = shade_skin(
            &albedo,
            &skin_params(),
            &skin_level(Quality::Potato),
            &normal,
            &light,
            &view,
        );
        let on = shade_skin(
            &albedo,
            &skin_params(),
            &skin_level(Quality::Comfort),
            &normal,
            &light,
            &view,
        );
        assert!(off.is_ok());
        assert!(on.is_ok());
        if let (Ok(bare), Ok(full)) = (off, on) {
            assert!(approx(bare, [0.0, 0.0, 0.0]));
            assert!(approx(full, [0.54, 0.24, 0.18]));
            assert!(full[0] > bare[0]);
        }
    }

    #[test]
    fn eye_glint_peaks_head_on() {
        // Light, normal and viewer aligned: half-vector dot is one, so the
        // glint adds its full strength: [0.2 + 0.8; 3] = [1.0; 3].
        let albedo = [0.2, 0.2, 0.2];
        let normal = [0.0, 0.0, 1.0];
        let view = [0.0, 0.0, 1.0];
        let head = shade_eye(
            &albedo,
            &eye_params(),
            &skin_level(Quality::High),
            &normal,
            &[0.0, 0.0, 1.0],
            &view,
        );
        let edge = shade_eye(
            &albedo,
            &eye_params(),
            &skin_level(Quality::High),
            &normal,
            &[1.0, 0.0, 0.0],
            &view,
        );
        let gated = shade_eye(
            &albedo,
            &eye_params(),
            &skin_level(Quality::Comfort),
            &normal,
            &[0.0, 0.0, 1.0],
            &view,
        );
        assert!(head.is_ok());
        assert!(edge.is_ok());
        assert!(gated.is_ok());
        if let (Ok(h), Ok(e), Ok(g)) = (head, edge, gated) {
            assert!(approx(h, [1.0, 1.0, 1.0]));
            assert!(e[0] <= 1e-6);
            assert!(h[0] > e[0] + 0.5);
            assert!(approx(g, [0.2, 0.2, 0.2]));
        }
    }

    #[test]
    fn hair_band_peaks_across_strand() {
        // Light and viewer overhead: strands across the half-vector catch
        // the full band, strands along it catch nothing.
        let albedo = [0.3, 0.2, 0.15];
        let normal = [0.0, 0.0, 1.0];
        let light = [0.0, 0.0, 1.0];
        let view = [0.0, 0.0, 1.0];
        let across = shade_hair(
            &albedo,
            &hair_params(),
            &skin_level(Quality::High),
            &normal,
            &[1.0, 0.0, 0.0],
            &light,
            &view,
        );
        let along = shade_hair(
            &albedo,
            &hair_params(),
            &skin_level(Quality::High),
            &normal,
            &[0.0, 0.0, 1.0],
            &light,
            &view,
        );
        assert!(across.is_ok());
        assert!(along.is_ok());
        if let (Ok(a), Ok(b)) = (across, along) {
            assert!(approx(a, [0.78, 0.62, 0.51]));
            assert!(approx(b, albedo));
            assert!(a[0] > b[0] + 0.4);
        }
    }

    #[test]
    fn rejects_bad_inputs_loudly() {
        let ok_n = [0.0, 0.0, 1.0];
        let zero = [0.0, 0.0, 0.0];
        assert_eq!(
            shade_skin(
                &[0.5, 0.5, 0.5],
                &skin_params(),
                &skin_level(Quality::High),
                &zero,
                &ok_n,
                &ok_n,
            ),
            Err(ShadeError::Degenerate("skin.normal"))
        );
        assert_eq!(
            shade_skin(
                &[f32::NAN, 0.5, 0.5],
                &skin_params(),
                &skin_level(Quality::High),
                &ok_n,
                &ok_n,
                &ok_n,
            ),
            Err(ShadeError::NonFinite("skin.albedo"))
        );
        assert_eq!(
            SkinParams {
                wrap: 2.0,
                ..skin_params()
            }
            .validate(),
            Err(ShadeError::OutOfRange("skin.wrap"))
        );
        assert_eq!(
            EyeParams {
                glint_sharp: 0,
                ..eye_params()
            }
            .validate(),
            Err(ShadeError::OutOfRange("eye.glint_sharp"))
        );
        assert_eq!(
            HairParams {
                shift: 2.0,
                ..hair_params()
            }
            .validate(),
            Err(ShadeError::OutOfRange("hair.shift"))
        );
        assert_eq!(
            shade_hair(
                &[0.3, 0.2, 0.15],
                &hair_params(),
                &skin_level(Quality::High),
                &ok_n,
                &zero,
                &ok_n,
                &ok_n,
            ),
            Err(ShadeError::Degenerate("hair.tangent"))
        );
    }

    #[test]
    fn wgsl_holds_slice_terms_only() {
        assert!(SKIN_WGSL.contains("skin_shade"));
        assert!(SKIN_WGSL.contains("eye_shade"));
        assert!(SKIN_WGSL.contains("hair_shade"));
        assert!(!SKIN_WGSL.contains("aces"));
        assert!(!SKIN_WGSL.contains("direct_light"));
        assert!(!SKIN_WGSL.contains("tone"));
    }

    #[test]
    fn eval_bit_identical_twice() {
        let albedo = [0.55, 0.35, 0.28];
        let normal = [0.0, 0.3, 1.0];
        let light = [0.2, 0.4, 0.9];
        let view = [0.0, 0.0, 1.0];
        let level = skin_level(Quality::High);
        assert_eq!(
            shade_skin(&albedo, &skin_params(), &level, &normal, &light, &view),
            shade_skin(&albedo, &skin_params(), &level, &normal, &light, &view)
        );
        assert_eq!(
            shade_eye(&albedo, &eye_params(), &level, &normal, &light, &view),
            shade_eye(&albedo, &eye_params(), &level, &normal, &light, &view)
        );
        assert_eq!(
            shade_hair(
                &albedo,
                &hair_params(),
                &level,
                &normal,
                &[1.0, 0.0, 0.0],
                &light,
                &view,
            ),
            shade_hair(
                &albedo,
                &hair_params(),
                &level,
                &normal,
                &[1.0, 0.0, 0.0],
                &light,
                &view,
            )
        );
    }

    #[test]
    fn error_display_names_lane() {
        assert!(format!("{}", ShadeError::NonFinite("skin.wrap")).contains("skin.wrap"));
        assert!(format!("{}", ShadeError::Degenerate("eye.view")).contains("eye.view"));
        assert!(format!("{}", ShadeError::OutOfRange("hair.shift")).contains("hair.shift"));
    }
}
