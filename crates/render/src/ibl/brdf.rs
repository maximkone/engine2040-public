//! brdf: the GGX split-sum scale/bias LUT built from Hammersley importance taps.

use super::*;

// ---------------------------------------------------------------------------
// BRDF LUT: GGX split-sum scale/bias over (NdotV, roughness).
// ---------------------------------------------------------------------------

/// Split-sum BRDF LUT: texel `(ix, iy)` holds the `(scale, bias)` pair for
/// `NdotV = (ix + 0.5) / size`, `roughness = (iy + 0.5) / size`, integrated
/// with [`BRDF_SAMPLES`] Hammersley taps (Epic/Karis GGX importance sample).
/// Row-major, `size * size` texels.
#[derive(Clone, PartialEq, Debug)]
pub struct BrdfLut {
    /// LUT edge in texels.
    pub size: u32,
    /// Row-major `(scale, bias)` pairs.
    pub data: Vec<[f32; 2]>,
}

impl BrdfLut {
    /// Zero-filled LUT (test backing, never a cook result).
    pub fn new(size: u32) -> Self {
        Self {
            size,
            data: vec![[0.0; 2]; size as usize * size as usize],
        }
    }

    /// Texel count.
    pub fn texels(&self) -> usize {
        self.data.len()
    }

    /// Read one texel; bounds-checked for callers outside the bake.
    pub fn texel(&self, x: u32, y: u32) -> Result<[f32; 2], IblError> {
        if x >= self.size || y >= self.size {
            return Err(IblError::Invalid(format!(
                "ibl brdf texel ({x},{y}) outside {}x{}",
                self.size, self.size
            )));
        }
        Ok(self.data[y as usize * self.size as usize + x as usize])
    }

    /// Deterministic little-endian bytes: row-major, `(scale, bias)` order.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len() * 8);
        for t in &self.data {
            for c in t {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out
    }
}

pub(crate) fn radical_inverse_vdc(mut bits: u32) -> f32 {
    // Van der Corput base 2: a 16-bit half swap first. A byte swap here
    // rotates the sequence by 8 bits, and with 128 taps every sample lands
    // near zero, so the LUT would claim a roughness-1 lobe loses no energy.
    bits = bits.rotate_left(16);
    bits = ((bits & 0x5555_5555) << 1) | ((bits & 0xAAAA_AAAA) >> 1);
    bits = ((bits & 0x3333_3333) << 2) | ((bits & 0xCCCC_CCCC) >> 2);
    bits = ((bits & 0x0F0F_0F0F) << 4) | ((bits & 0xF0F0_F0F0) >> 4);
    bits = ((bits & 0x00FF_00FF) << 8) | ((bits & 0xFF00_FF00) >> 8);
    (bits as f64 * 2.328_306_436_538_696_3e-10) as f32
}

fn hammersley(i: u32, n: u32) -> [f32; 2] {
    [i as f32 / n as f32, radical_inverse_vdc(i)]
}

/// GGX importance-sampled split-sum integral for one `(NdotV, roughness)`:
/// returns `(scale, bias)` so `spec = prefiltered * (F0 * scale + bias)`.
/// Fixed Hammersley sequence, fixed tap count: bit-identical per input.
pub fn brdf_integrate(ndotv: f32, roughness: f32) -> Result<[f32; 2], IblError> {
    brdf_integrate_taps(ndotv, roughness, BRDF_SAMPLES)
}

/// [`brdf_integrate`] with an explicit Hammersley tap count (RL47 A7: the
/// table's tap count is a measured trade, so the measurement can run both).
/// Zero taps refuse; a power of two keeps the Van der Corput set evenly spread.
pub fn brdf_integrate_taps(ndotv: f32, roughness: f32, taps: u32) -> Result<[f32; 2], IblError> {
    if taps == 0 {
        return Err(IblError::Invalid("ibl brdf needs at least one tap".into()));
    }
    if !ndotv.is_finite() || !(0.0..=1.0).contains(&ndotv) {
        let reason = format!("ibl NdotV {ndotv} outside 0..=1"); // alloc-ok: cold refusal at the bake seam, never a steady frame
        return Err(IblError::Invalid(reason));
    }
    let roughness = check_roughness(roughness)?;
    let ndotv = ndotv.max(1e-4);
    let vx = (1.0 - ndotv * ndotv).max(0.0).sqrt();
    let v = [vx, 0.0, ndotv];
    let a = roughness * roughness;
    let a2 = a * a;
    // Schlick-GGX IBL geometry factor `k = alpha / 2` with
    // `alpha = roughness^2` (Karis course notes; the NDF above squares it
    // again, the geometry term does not).
    let k = a * 0.5;
    let ggx = |ndot: f32| ndot / (ndot * (1.0 - k) + k).max(1e-9);
    let mut acc = [0.0f64; 2];
    for i in 0..taps {
        let xi = hammersley(i, taps);
        let phi = 2.0 * PI * xi[0];
        let cos_theta = ((1.0 - xi[1]) / (1.0 + (a2 - 1.0) * xi[1]).max(1e-9)).sqrt();
        let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
        let h = [sin_theta * phi.cos(), sin_theta * phi.sin(), cos_theta];
        let vdot_h = (v[0] * h[0] + v[1] * h[1] + v[2] * h[2]).max(0.0);
        let ndot_h = h[2].max(0.0);
        let ndot_l = (2.0 * vdot_h * h[2] - v[2]).max(0.0);
        if ndot_l > 0.0 {
            let g = ggx(ndotv) * ggx(ndot_l);
            let g_vis = (g * vdot_h / (ndot_h * ndotv).max(1e-9)) as f64;
            let fc = (1.0 - vdot_h).powi(5) as f64;
            acc[0] += (1.0 - fc) * g_vis;
            acc[1] += fc * g_vis;
        }
    }
    let n = f64::from(taps);
    Ok([(acc[0] / n) as f32, (acc[1] / n) as f32])
}

/// Full LUT bake over the `(NdotV, roughness)` grid: texel centers, fixed
/// order. Bit-identical per size on every run.
pub fn brdf_lut_build(size: u32) -> Result<BrdfLut, IblError> {
    brdf_lut_build_taps(size, BRDF_SAMPLES)
}

/// [`brdf_lut_build`] with an explicit tap count per texel.
pub fn brdf_lut_build_taps(size: u32, taps: u32) -> Result<BrdfLut, IblError> {
    if size == 0 || size > 512 {
        return Err(IblError::Invalid(format!(
            "ibl brdf size {size} outside 1..=512"
        )));
    }
    let mut lut = BrdfLut::new(size);
    for y in 0..size {
        for x in 0..size {
            let ndotv = (x as f32 + 0.5) / size as f32;
            let roughness = (y as f32 + 0.5) / size as f32;
            lut.data[y as usize * size as usize + x as usize] =
                brdf_integrate_taps(ndotv, roughness, taps)?;
        }
    }
    Ok(lut)
}

/// Bilinear sample of the LUT: edges clamp, so `NdotV = 0/1` stays defined.
pub fn brdf_sample(lut: &BrdfLut, ndotv: f32, roughness: f32) -> Result<[f32; 2], IblError> {
    if !ndotv.is_finite() || !(0.0..=1.0).contains(&ndotv) {
        return Err(IblError::Invalid(format!(
            "ibl NdotV {ndotv} outside 0..=1"
        )));
    }
    let r = check_roughness(roughness)?;
    if lut.size == 0 {
        return Err(IblError::Invalid("ibl brdf lut is empty".into()));
    }
    let w = lut.size as f32;
    let x = (ndotv * w - 0.5).clamp(0.0, w - 1.0);
    let y = (r * w - 0.5).clamp(0.0, w - 1.0);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(lut.size - 1);
    let y1 = (y0 + 1).min(lut.size - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let n = lut.size as usize;
    let at = |xx: u32, yy: u32| lut.data[yy as usize * n + xx as usize];
    let a = at(x0, y0);
    let b = at(x1, y0);
    let c = at(x0, y1);
    let e = at(x1, y1);
    Ok([
        a[0] + ((b[0] - a[0]) * fx + (c[0] - a[0]) * fy + (a[0] - b[0] - c[0] + e[0]) * fx * fy),
        a[1] + ((b[1] - a[1]) * fx + (c[1] - a[1]) * fy + (a[1] - b[1] - c[1] + e[1]) * fx * fy),
    ])
}

// ---------------------------------------------------------------------------
// Cook-time bake behind `v1`: HDRI bytes in, deterministic bake out.
// ---------------------------------------------------------------------------
