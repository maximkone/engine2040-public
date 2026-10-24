//! bake: the cook receipt, the roughness-stepped prefiltered cubemap, its sampling
//! and the cosine-convolved irradiance SH. Std-only, f64 accumulation, fixed orders.

use super::*;

// ---------------------------------------------------------------------------
// Receipts: texel counts plus wall-clock bake ms for the frame report.
// ---------------------------------------------------------------------------

/// One IBL cook receipt: what baked, how big, how long.
#[derive(Clone, PartialEq, Debug)]
pub struct IblReceipt {
    /// Prefiltered texels resident (`levels * 6 * size * size`).
    pub cube_texels: u64,
    /// BRDF LUT texels resident (`size * size`).
    pub lut_texels: u64,
    /// SH coefficients resident (always [`IBL_SH_COEFFS`]).
    pub sh_coeffs: u32,
    /// Wall-clock bake time in ms; evidence only, never hashed.
    pub ms: f64,
}

// ---------------------------------------------------------------------------
// Prefiltered cubemap: levels of roughness-blurred radiance faces.
// ---------------------------------------------------------------------------

/// Roughness-stepped cubemap: level `l` blurs with roughness
/// `l / (LEVELS - 1)` per the R2 `crate::v1_ibl_shadow::prefilter` kernel.
/// Layout is level-major, face-major (`+X,-X,+Y,-Y,+Z,-Z`), row-major texels.
#[derive(Clone, PartialEq, Debug)]
pub struct PrefilterCube {
    /// Face edge in texels.
    pub size: u32,
    /// Roughness levels.
    pub levels: usize,
    /// `levels * 6 * size * size` linear radiance texels.
    pub faces: Vec<[f32; 3]>,
}

impl PrefilterCube {
    /// Zero-filled cube (test and placeholder backing, never a cook result).
    pub fn new(size: u32, levels: usize) -> Self {
        Self {
            size,
            levels,
            faces: vec![[0.0; 3]; levels * IBL_FACES * size as usize * size as usize],
        }
    }

    /// Texel count across all levels and faces.
    pub fn texels(&self) -> usize {
        self.faces.len()
    }

    /// Roughness stored at `level`.
    pub fn level_roughness(level: usize) -> Result<f32, IblError> {
        if level >= IBL_LEVELS {
            return Err(IblError::Invalid(format!(
                "ibl level {level} outside 0..{IBL_LEVELS}"
            )));
        }
        Ok(level as f32 / (IBL_LEVELS - 1) as f32)
    }

    fn face_base(&self, level: usize, face: usize) -> Result<usize, IblError> {
        if level >= self.levels {
            return Err(IblError::Invalid(format!(
                "ibl level {level} outside 0..{}",
                self.levels
            )));
        }
        if face >= IBL_FACES {
            return Err(IblError::Invalid(format!(
                "ibl face {face} outside 0..{IBL_FACES}"
            )));
        }
        Ok((level * IBL_FACES + face) * self.size as usize * self.size as usize)
    }

    /// Read one texel; bounds-checked for callers outside the bake.
    pub fn texel(&self, level: usize, face: usize, x: u32, y: u32) -> Result<[f32; 3], IblError> {
        if x >= self.size || y >= self.size {
            return Err(IblError::Invalid(format!(
                "ibl texel ({x},{y}) outside {}x{}",
                self.size, self.size
            )));
        }
        let base = self.face_base(level, face)?;
        Ok(self.faces[base + y as usize * self.size as usize + x as usize])
    }

    /// Deterministic little-endian bytes: level-major, face-major,
    /// row-major texels, RGB component order.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.faces.len() * 12);
        for t in &self.faces {
            for c in t {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out
    }
}

/// Unit direction through a texel center. Row 0 is the top (`-v`), matching
/// the image convention; the exact orientation is engine-internal, but bake
/// and sample share it, so round-trips are exact.
pub fn cube_texel_dir(face: usize, x: u32, y: u32, size: u32) -> [f32; 3] {
    let u = (2.0 * (x as f32 + 0.5) / size as f32) - 1.0;
    let v = (2.0 * (y as f32 + 0.5) / size as f32) - 1.0;
    let d = match face {
        0 => [1.0, -v, -u],
        1 => [-1.0, -v, u],
        2 => [u, 1.0, v],
        3 => [u, -1.0, -v],
        4 => [u, -v, 1.0],
        _ => [-u, -v, -1.0],
    };
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    [d[0] / l, d[1] / l, d[2] / l]
}

pub(crate) fn check_unit_dir(dir: [f32; 3], what: &'static str) -> Result<[f32; 3], IblError> {
    if !dir.iter().all(|c| c.is_finite()) {
        return Err(IblError::Invalid(format!("ibl {what} is not finite")));
    }
    let l2 = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2];
    if l2 < 1e-12 {
        return Err(IblError::Invalid(format!("ibl {what} has ~zero length")));
    }
    let l = l2.sqrt();
    Ok([dir[0] / l, dir[1] / l, dir[2] / l])
}

pub(crate) fn check_roughness(roughness: f32) -> Result<f32, IblError> {
    if !roughness.is_finite() || !(0.0..=1.0).contains(&roughness) {
        return Err(IblError::Invalid(format!(
            "ibl roughness {roughness} outside 0..=1"
        )));
    }
    Ok(roughness)
}

/// Bilinear sample of one cube level for `dir`: dominant axis picks the face
/// (the inverse of [`cube_texel_dir`]), UV clamps at face edges. Level blends
/// toward `roughness`: `f = roughness * (levels - 1)` lerps the two nearest
/// stored levels, so stored roughness steps reproduce the bake exactly.
pub fn sample_cube(
    cube: &PrefilterCube,
    dir: [f32; 3],
    roughness: f32,
) -> Result<[f32; 3], IblError> {
    let d = check_unit_dir(dir, "sample direction")?;
    let r = check_roughness(roughness)?;
    if cube.levels == 0 || cube.size == 0 {
        return Err(IblError::Invalid("ibl cube is empty".into()));
    }
    let f = r * (cube.levels - 1) as f32;
    let l0 = (f.floor() as usize).min(cube.levels - 1);
    let l1 = (l0 + 1).min(cube.levels - 1);
    let t = f - l0 as f32;
    let a = sample_level(cube, l0, d)?;
    if l1 == l0 {
        return Ok(a);
    }
    let b = sample_level(cube, l1, d)?;
    Ok([
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ])
}

fn sample_level(cube: &PrefilterCube, level: usize, d: [f32; 3]) -> Result<[f32; 3], IblError> {
    let ax = d[0].abs();
    let ay = d[1].abs();
    let az = d[2].abs();
    // Dominant axis selects the face; the (u, v) projections invert
    // `cube_texel_dir` exactly, so texel centers round-trip bit-for-bit.
    let (face, u, v): (usize, f32, f32) = if ax >= ay && ax >= az {
        if d[0] > 0.0 {
            (0, -d[2] / ax, -d[1] / ax)
        } else {
            (1, d[2] / ax, -d[1] / ax)
        }
    } else if ay >= ax && ay >= az {
        if d[1] > 0.0 {
            (2, d[0] / ay, d[2] / ay)
        } else {
            (3, d[0] / ay, -d[2] / ay)
        }
    } else if d[2] > 0.0 {
        (4, d[0] / az, -d[1] / az)
    } else {
        (5, -d[0] / az, -d[1] / az)
    };
    let size = cube.size;
    let x = ((u + 1.0) * 0.5 * size as f32 - 0.5).clamp(0.0, size as f32 - 1.0);
    let y = ((v + 1.0) * 0.5 * size as f32 - 0.5).clamp(0.0, size as f32 - 1.0);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(size - 1);
    let y1 = (y0 + 1).min(size - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let base = cube.face_base(level, face)?;
    let n = size as usize;
    let at = |xx: u32, yy: u32| cube.faces[base + yy as usize * n + xx as usize];
    let a = at(x0, y0);
    let b = at(x1, y0);
    let c = at(x0, y1);
    let e = at(x1, y1);
    let mix = |p: [f32; 3], q: [f32; 3], t: f32| {
        [
            p[0] + (q[0] - p[0]) * t,
            p[1] + (q[1] - p[1]) * t,
            p[2] + (q[2] - p[2]) * t,
        ]
    };
    Ok(mix(mix(a, b, fx), mix(c, e, fx), fy))
}

// ---------------------------------------------------------------------------
// Irradiance SH: cosine-convolved projection shared with `crate::gi`.
// ---------------------------------------------------------------------------

/// Irradiance spherical harmonics: 9 RGB coefficients convolved with the
/// clamped-cosine lobe (`A0 = PI`, `A1 = 2PI/3`, `A2 = PI/4` per Ram-Ng), so
/// `eval_sh` returns irradiance directly. The basis is
/// [`crate::gi::sh_basis`]; bake and eval share it by construction.
#[derive(Clone, PartialEq, Debug)]
pub struct IrradianceSh {
    /// `A_l`-convolved RGB coefficients, L0..L2 order of `gi::sh_basis`.
    pub coeffs: [[f32; 3]; IBL_SH_COEFFS],
}

impl IrradianceSh {
    /// Zero coefficients (test backing, never a cook result).
    pub fn zero() -> Self {
        Self {
            coeffs: [[0.0; 3]; IBL_SH_COEFFS],
        }
    }

    /// Deterministic little-endian bytes: coefficient-major, RGB order.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(IBL_SH_COEFFS * 12);
        for c in &self.coeffs {
            for v in c {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }
}

/// Diffuse irradiance for `normal` from convolved coefficients: the same dot
/// product as [`crate::gi::sh_irradiance`], so probe and IBL eval agree.
pub fn eval_sh(sh: &IrradianceSh, normal: [f32; 3]) -> Result<[f32; 3], IblError> {
    let n = check_unit_dir(normal, "sh normal")?;
    let rgb = crate::gi::sh_irradiance(&sh.coeffs, n);
    Ok([rgb[0].max(0.0), rgb[1].max(0.0), rgb[2].max(0.0)])
}

/// Baked ambient scaled by occlusion: `eval_sh` times `occlusion`.
///
/// Owner: render lane; consumer: `shade_ibl` (open 1.0 here) plus the
/// forward shader's baked term, which the live full-res value scales the
/// same way. Units: linear irradiance with `occlusion` unitless `0..=1`
/// (`1` open, `0` fully shadowed). Determinism: same inputs give the same
/// triple on every run; no allocation, no clock. Errors:
/// [`IblError::Invalid`] when `occlusion` is outside `0..=1` or not finite,
/// or when `normal` is not a finite nonzero direction.
pub fn ambient_occluded(
    sh: &IrradianceSh,
    normal: [f32; 3],
    occlusion: f32,
) -> Result<[f32; 3], IblError> {
    if !occlusion.is_finite() || !(0.0..=1.0).contains(&occlusion) {
        return Err(IblError::Invalid(format!(
            "ibl occlusion {occlusion} outside 0..=1"
        )));
    }
    let e = eval_sh(sh, normal)?;
    Ok([e[0] * occlusion, e[1] * occlusion, e[2] * occlusion])
}
