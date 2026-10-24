//! light: `IblBake`, the deterministic cook from one HDRI into cube, SH and LUT.

use super::*;

/// Cooked IBL: prefiltered cubemap plus irradiance SH plus BRDF LUT, baked
/// from one HDRI. `to_bytes` is the deterministic cook artifact (level-major
/// cube, then SH, then LUT); same HDRI bytes give bit-identical bakes.
#[cfg(feature = "v1")]
#[derive(Clone, PartialEq, Debug)]
pub struct IblBake {
    /// Roughness-stepped prefiltered cubemap.
    pub cube: PrefilterCube,
    /// Cosine-convolved irradiance SH.
    pub sh: IrradianceSh,
    /// GGX split-sum LUT.
    pub brdf: BrdfLut,
    /// Mean HDRI texel (exposure calibration, the R2 `env_average`).
    pub source_average: [f32; 3],
}

#[cfg(feature = "v1")]
impl IblBake {
    /// Receipt for the last bake (pass ms supplied by the caller).
    pub fn receipt(&self, ms: f64) -> IblReceipt {
        IblReceipt {
            cube_texels: self.cube.texels() as u64,
            lut_texels: self.brdf.texels() as u64,
            sh_coeffs: IBL_SH_COEFFS as u32,
            ms,
        }
    }

    /// Deterministic cook artifact bytes: cube, then SH, then LUT.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.cube.to_bytes();
        out.extend_from_slice(&self.sh.to_bytes());
        out.extend_from_slice(&self.brdf.to_bytes());
        out
    }

    /// Headless submit seam (no device in this build): validates the shared
    /// [`crate::gtao::LookFrame`], then refuses loudly with the reason. The
    /// CPU bake above is the reference here; the `window` build below
    /// records the live pass instead. Never silent success.
    #[cfg(all(feature = "v1", not(feature = "gpu")))]
    pub fn submit_gpu(&self, frame: &crate::gtao::LookFrame) -> Result<IblReceipt, IblError> {
        frame
            .validate()
            .map_err(|e| IblError::Invalid(format!("ibl look frame: {e}")))?; // alloc-ok: cold refusal, never a steady frame
        Err(IblError::Invalid(
            "ibl gpu pass needs the window build with a live device; CPU bake is the reference"
                .into(),
        ))
    }
}

/// Decode HDRI bytes and bake the full IBL set. Empty bytes refuse with
/// [`IblError::MissingHdri`]; undecodable bytes wrap the R2 error. The same
/// complete bytes give bit-identical bakes on every run.
#[cfg(feature = "v1")]
pub fn bake(hdri_bytes: &[u8]) -> Result<(IblBake, f64), IblError> {
    if hdri_bytes.is_empty() {
        return Err(IblError::MissingHdri);
    }
    let started = engine_clock::now();
    let env = crate::v1_ibl_shadow::load_hdri(hdri_bytes).map_err(IblError::from)?;
    let (bake, _) = bake_env(&env)?;
    let ms = engine_clock::since(started).as_secs_f64() * 1000.0;
    Ok((bake, ms))
}

/// Bake the full IBL set from an already built environment: `bake` after
/// its decode, and the arena day sky (`ibl_arena_env`) which has no file. The
/// same map gives a bit-identical bake on every run. Returns the bake plus the
/// wall ms (evidence only).
#[cfg(feature = "v1")]
pub fn bake_env(env: &crate::v1_ibl_shadow::EnvMap) -> Result<(IblBake, f64), IblError> {
    let started = engine_clock::now();
    let cube = bake_cube(env)?;
    let sh = project_sh(env);
    let brdf = brdf_lut_build(BRDF_LUT_SIZE)?;
    let source_average = crate::v1_ibl_shadow::env_average(env);
    let ms = engine_clock::since(started).as_secs_f64() * 1000.0;
    Ok((
        IblBake {
            cube,
            sh,
            brdf,
            source_average,
        },
        ms,
    ))
}

/// Prefilter every (level, face, texel) through the R2 roughness kernel:
/// parity with the R2 CPU reference is exact by construction.
#[cfg(feature = "v1")]
fn bake_cube(env: &crate::v1_ibl_shadow::EnvMap) -> Result<PrefilterCube, IblError> {
    let mut cube = PrefilterCube::new(IBL_CUBE_SIZE, IBL_LEVELS);
    for level in 0..IBL_LEVELS {
        let roughness = PrefilterCube::level_roughness(level)?;
        for face in 0..IBL_FACES {
            for y in 0..IBL_CUBE_SIZE {
                for x in 0..IBL_CUBE_SIZE {
                    let dir = cube_texel_dir(face, x, y, IBL_CUBE_SIZE);
                    let tap = crate::v1_ibl_shadow::prefilter(env, dir, roughness)
                        .map_err(IblError::from)?;
                    let idx = (level * IBL_FACES + face)
                        * IBL_CUBE_SIZE as usize
                        * IBL_CUBE_SIZE as usize
                        + y as usize * IBL_CUBE_SIZE as usize
                        + x as usize;
                    cube.faces[idx] = tap;
                }
            }
        }
    }
    Ok(cube)
}

/// Solid-angle-weighted SH projection with the `gi` basis, convolved with
/// the clamped-cosine lobe. f64 accumulation in fixed texel order.
#[cfg(feature = "v1")]
pub(crate) fn project_sh(env: &crate::v1_ibl_shadow::EnvMap) -> IrradianceSh {
    const A0: f64 = std::f64::consts::PI;
    const A1: f64 = 2.0 * std::f64::consts::PI / 3.0;
    const A2: f64 = std::f64::consts::PI / 4.0;
    const LOBE: [f64; IBL_SH_COEFFS] = [A0, A1, A1, A1, A2, A2, A2, A2, A2];
    let w = env.width as usize;
    let h = env.height as usize;
    let d_phi = std::f64::consts::PI / h as f64;
    let d_theta = 2.0 * std::f64::consts::PI / w as f64;
    let mut acc = [[0.0f64; 3]; IBL_SH_COEFFS];
    for y in 0..h {
        // Texel-center solid angle: sin(phi) * dphi * dtheta.
        let phi = (y as f64 + 0.5) / h as f64 * std::f64::consts::PI;
        let domega = phi.sin() * d_phi * d_theta;
        for x in 0..w {
            let p = env.pixels[y * w + x];
            let dir = crate::v1_ibl_shadow::uv_to_dir([
                (x as f32 + 0.5) / env.width as f32,
                (y as f32 + 0.5) / env.height as f32,
            ]);
            let b = crate::gi::sh_basis(dir);
            for i in 0..IBL_SH_COEFFS {
                acc[i][0] += f64::from(p[0]) * f64::from(b[i]) * domega;
                acc[i][1] += f64::from(p[1]) * f64::from(b[i]) * domega;
                acc[i][2] += f64::from(p[2]) * f64::from(b[i]) * domega;
            }
        }
    }
    let mut coeffs = [[0.0f32; 3]; IBL_SH_COEFFS];
    for i in 0..IBL_SH_COEFFS {
        coeffs[i] = [
            (acc[i][0] * LOBE[i]) as f32,
            (acc[i][1] * LOBE[i]) as f32,
            (acc[i][2] * LOBE[i]) as f32,
        ];
    }
    IrradianceSh { coeffs }
}
