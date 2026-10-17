//! post jitter and motion leaf: Halton-2/3 per-frame projection jitter plus
//! current-minus-history motion vectors for the TAA resolve.
//!
//! Owner: render lane; the [`crate::post`] facade re-exports every public
//! item here with `pub use crate::post_jitter_motion::*;`, so callers keep the
//! `post::` path. Units: jitter offsets and motion vectors in pixels, NDC for
//! the projection add; the frame number is the only time input. Determinism:
//! integer frame and size math with fixed Halton bases 2 and 3, so the same
//! frame and resolution give bit-identical offsets and wall clock never
//! enters. Budget: image sizes are validated against [`MAX_POST_DIM`] before
//! allocation and bad input fails with a typed [`PostError`] (whose
//! `BudgetExceeded` carries the allowed bound), never silent success.

use crate::post::{PostError, MAX_POST_DIM};

// ---------------------------------------------------------------------------
// Jittered projection: Halton-2/3 per-frame offsets (engine-owned).
// ---------------------------------------------------------------------------

/// Radical inverse of `index` in `base`. Pure textbook math; frame callers
/// pass `frame + 1` so frame 0 still jitters off the origin.
pub fn halton(mut index: u32, base: u32) -> f32 {
    debug_assert!(base >= 2, "halton base must be >= 2");
    let (b, mut denom) = (f64::from(base), 1.0f64);
    let mut out = 0.0f64;
    while index > 0 {
        denom *= b;
        out += f64::from(index % base) / denom;
        index /= base;
    }
    out as f32
}

/// Jitter offset for `frame` in pixels, centered on `[-0.5, 0.5)`: x from
/// Halton base 2, y from Halton base 3. Depends only on the frame number,
/// never on wall time. The single owner of the R4 Halton-2/3 sequence
/// (the `v1` parity test pins the composition).
pub fn jitter_offset(frame: u64) -> [f32; 2] {
    let i = frame.wrapping_add(1) as u32;
    [halton(i, 2) - 0.5, halton(i, 3) - 0.5]
}

/// Jitter for `frame` in NDC units (`offset * 2 / res`), the value
/// [`jitter_projection`] adds to the projection matrix. Zero resolution
/// (suspended swap path) yields zero jitter.
pub fn jitter_ndc(frame: u64, width: u32, height: u32) -> [f32; 2] {
    if width == 0 || height == 0 {
        return [0.0, 0.0];
    }
    let [jx, jy] = jitter_offset(frame);
    [jx * 2.0 / width as f32, jy * 2.0 / height as f32]
}

/// GPU uniform `[jx_px, jy_px, jx_ndc, jy_ndc]` for the resolve pass.
pub fn jitter_uniform(frame: u64, width: u32, height: u32) -> [f32; 4] {
    let [px, py] = jitter_offset(frame);
    let [nx, ny] = jitter_ndc(frame, width, height);
    [px, py, nx, ny]
}

/// Jitter a row-major projection matrix for this frame: offsets the
/// perspective terms (`m[2][0]`, `m[2][1]`), leaving everything else
/// bit-identical. The `v1` wrapper pins the same terms on `glam::Mat4`.
pub fn jitter_projection(proj: [[f32; 4]; 4], ndc: [f32; 2]) -> [[f32; 4]; 4] {
    let mut out = proj;
    out[2][0] += ndc[0];
    out[2][1] += ndc[1];
    out
}

/// Jitter a `glam` projection matrix (same terms as [`jitter_projection`])
/// for the `v1_gpu` TAA pass. Behind `v1` with the camera math.
#[cfg(feature = "v1")]
pub fn jitter_projection_glam(proj: glam::Mat4, ndc: [f32; 2]) -> glam::Mat4 {
    let mut out = proj;
    out.z_axis.x += ndc[0];
    out.z_axis.y += ndc[1];
    out
}

// ---------------------------------------------------------------------------
// Motion vectors: current-minus-history flow in pixels.
// ---------------------------------------------------------------------------

/// Per-pixel motion in pixels: `motion[x, y] = current_pos - history_pos`,
/// so the history sample for `(x, y)` sits at `(x - mx, y - my)`.
/// Row-major, `width * height` entries.
#[derive(Clone, PartialEq, Debug)]
pub struct MotionField {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Motion vectors in pixels, row-major.
    pub flow: Vec<[f32; 2]>,
}

impl MotionField {
    /// Build from an explicit flow field: validates dims, length and
    /// finiteness before the resolve can read it.
    pub fn build(width: u32, height: u32, flow: Vec<[f32; 2]>) -> Result<Self, PostError> {
        check_dims(width, height)?;
        let want = width as usize * height as usize;
        if flow.len() != want {
            return Err(PostError::Invalid(format!(
                "motion flow {} differs from {width}x{height} ({want})",
                flow.len()
            )));
        }
        for (i, v) in flow.iter().enumerate() {
            if !v.iter().all(|c| c.is_finite()) {
                return Err(PostError::Invalid(format!(
                    "motion vector {i} is not finite"
                )));
            }
        }
        Ok(Self {
            width,
            height,
            flow,
        })
    }

    /// Zero motion everywhere (static camera): history reprojects 1:1.
    pub fn zero(width: u32, height: u32) -> Result<Self, PostError> {
        check_dims(width, height)?;
        Ok(Self {
            width,
            height,
            flow: vec![[0.0, 0.0]; width as usize * height as usize],
        })
    }

    /// Motion at texel `(x, y)` in pixels.
    pub fn at(&self, x: u32, y: u32) -> [f32; 2] {
        self.flow[y as usize * self.width as usize + x as usize]
    }

    /// Pixel count (`width * height`).
    pub fn len(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Whether the field holds no texels (only when a dimension is 0,
    /// which [`Self::build`] never produces).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub(crate) fn check_dims(width: u32, height: u32) -> Result<(), PostError> {
    if width == 0 || height == 0 {
        return Err(PostError::Invalid("post frame size is zero".into()));
    }
    if width > MAX_POST_DIM || height > MAX_POST_DIM {
        return Err(PostError::BudgetExceeded {
            requested: width.max(height),
            allowed: MAX_POST_DIM,
        });
    }
    Ok(())
}

pub(crate) fn check_image(
    pixels: &[[f32; 3]],
    width: u32,
    height: u32,
    what: &'static str,
) -> Result<(), PostError> {
    check_dims(width, height)?;
    let want = width as usize * height as usize;
    if pixels.len() != want {
        return Err(PostError::Invalid(format!(
            "{what} pixels {} differ from {width}x{height} ({want})",
            pixels.len()
        )));
    }
    for (i, p) in pixels.iter().enumerate() {
        if !p.iter().all(|c| c.is_finite()) {
            return Err(PostError::Invalid(format!(
                "{what} texel {i} is not finite"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn jitter_matches_halton_values() {
        // Textbook anchors: halton(1,2)=1/2, halton(2,2)=1/4, halton(1,3)=1/3.
        assert!(approx(halton(1, 2), 0.5, 1e-7));
        assert!(approx(halton(2, 2), 0.25, 1e-7));
        assert!(approx(halton(1, 3), 1.0 / 3.0, 1e-7));
        // Frame 0 leaves half-pixel x (0.5-0.5) and -1/6 y (1/3-1/2).
        let j0 = jitter_offset(0);
        assert!(approx(j0[0], 0.0, 1e-7));
        assert!(approx(j0[1], -1.0 / 6.0, 1e-6));
        for frame in [0u64, 1, 2, 7, 15] {
            let j = jitter_offset(frame);
            assert!(
                (-0.5..0.5).contains(&j[0]) && (-0.5..0.5).contains(&j[1]),
                "frame {frame} jitter {j:?} stays in the pixel"
            );
        }
        // NDC scales pixels by 2/res; zero res suspends to zero.
        let ndc = jitter_ndc(1, 1920, 1080);
        let [px, py] = jitter_offset(1);
        assert!(approx(ndc[0], px * 2.0 / 1920.0, 1e-7));
        assert!(approx(ndc[1], py * 2.0 / 1080.0, 1e-7));
        assert_eq!(jitter_ndc(3, 0, 720), [0.0, 0.0]);
        let u = jitter_uniform(1, 1280, 720);
        let [ex, ey] = jitter_ndc(1, 1280, 720);
        assert_eq!(u, [px, py, ex, ey]);
    }

    #[test]
    fn jittered_projection_touches_only_perspective_terms() {
        let proj = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 2.0, 0.0, 0.0],
            [0.3, -0.4, 1.0, 1.0],
            [0.0, 0.0, -0.5, 0.0],
        ];
        let out = jitter_projection(proj, [0.01, -0.02]);
        assert!(approx(out[2][0], 0.31, 1e-7));
        assert!(approx(out[2][1], -0.42, 1e-7));
        for r in [0, 1, 3] {
            assert_eq!(out[r], proj[r]);
        }
        assert_eq!(out[2][2], proj[2][2]);
        assert_eq!(out[2][3], proj[2][3]);
        assert_eq!(jitter_projection(proj, [0.0, 0.0]), proj);
    }

    #[test]
    fn motion_field_validates_inputs() {
        let m = MotionField::zero(4, 4).expect("zero builds");
        assert_eq!((m.width, m.height, m.len()), (4, 4, 16));
        assert!(!m.is_empty());
        assert_eq!(m.at(2, 1), [0.0, 0.0]);
        assert!(matches!(
            MotionField::zero(0, 4),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            MotionField::zero(4096, 4),
            Err(PostError::BudgetExceeded { .. })
        ));
        assert!(matches!(
            MotionField::build(2, 2, vec![[0.0, 0.0]; 3]),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            MotionField::build(2, 2, vec![[f32::NAN, 0.0]; 4]),
            Err(PostError::Invalid(_))
        ));
    }
}
