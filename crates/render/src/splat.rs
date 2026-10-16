//! Static Gaussian backdrop, presentation only.
//! Owner: backdrop rasterizer for licensed captures.
//! Units: metres for positions, pixels for tiles, far depths positive.
//! Determinism: fixed inputs give fixed tiles, no clock.
//! Errors: typed, no tick contact.
// PORT(ArthurBrussee/brush@1388f74c6fe0236f68ee4915564bf00e9d2e3747:camera pinhole plus covariance, tile bins, radix sort, serde layout, train config offline only): reimplemented, zero bytes vendored.
/// License row per capture, fixture-local.
/// Caller: tile first-pass presenter keeps one row per capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LicenseRow {
    pub source: &'static str,
    pub terms: &'static str,
    pub capture: &'static str,
}
/// Typed backdrop errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplatError {
    /// Missing or empty license row.
    MissingLicense,
    /// Bad PLY layout.
    BadPly,
    /// Bad tile or buffer sizes.
    BadDim,
}
impl core::fmt::Display for SplatError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
/// Presentation-only backdrop rasterizer.
/// Caller: tile first-pass presenter draws it with depth write off.
#[derive(Clone, Debug)]
pub struct SplatBackdrop {
    license: LicenseRow,
    cooked: Vec<u8>,
    fx: f32,
    fy: f32,
    cx: f32,
    cy: f32,
}
impl SplatBackdrop {
    /// Build from a PLY plus its license row.
    pub fn try_new(license: Option<LicenseRow>, ply: &[u8]) -> Result<Self, SplatError> {
        let lic = license
            .filter(|l| !l.source.is_empty() && !l.terms.is_empty() && !l.capture.is_empty())
            .ok_or(SplatError::MissingLicense)?;
        Ok(Self {
            license: lic,
            cooked: cook_ply(ply)?,
            fx: 64.0,
            fy: 64.0,
            cx: 32.0,
            cy: 32.0,
        })
    }
    /// Cooked byte length.
    pub fn cooked_len(&self) -> usize {
        self.cooked.len()
    }
    /// Capture tag.
    pub fn capture(&self) -> &'static str {
        self.license.capture
    }
    // 2D covariance from scale plus rotation; inputs here stay finite, so clamp holds.
    fn cov2d(sx: f32, sy: f32, qz: f32, qw: f32) -> [f32; 3] {
        let xx = sx * sx + 0.5;
        let lim = 0.9 * xx.min(sy * sy + 0.5);
        [xx, (qz * qw * sx * sy).clamp(-lim, lim), sy * sy + 0.5]
    }
    // One-pass radix sort over cooked depth bytes, front to back.
    fn sort_depth(cooked: &[u8]) -> Result<Vec<u32>, SplatError> {
        let n = cooked.len() / 5;
        let mut counts = [0u32; 256];
        let mut ord = Vec::with_capacity(n);
        for i in 0..n {
            ord.push(u32::try_from(i).map_err(|_| SplatError::BadDim)?);
            let b = usize::from(cooked[i * 5 + 2]);
            counts[b] = counts[b].wrapping_add(1);
        }
        let mut sum = 0u32;
        for c in counts.iter_mut() {
            let v = *c;
            *c = sum;
            sum = sum.wrapping_add(v);
        }
        let mut out = vec![0u32; n];
        for o in &ord {
            let oi = usize::try_from(*o).map_err(|_| SplatError::BadDim)?;
            let b = usize::from(cooked[oi * 5 + 2]);
            let at = usize::try_from(counts[b]).map_err(|_| SplatError::BadDim)?;
            *out.get_mut(at).ok_or(SplatError::BadDim)? = *o;
            counts[b] = counts[b].wrapping_add(1);
        }
        Ok(out)
    }
    /// Composite tile with depth test on and depth write off.
    pub fn composite(
        &self,
        tile: &mut [u8],
        arena: &[f32],
        w: u32,
        h: u32,
    ) -> Result<u64, SplatError> {
        let wu = usize::try_from(w).map_err(|_| SplatError::BadDim)?;
        let hu = usize::try_from(h).map_err(|_| SplatError::BadDim)?;
        let cells = wu.checked_mul(hu).ok_or(SplatError::BadDim)?;
        if w == 0 || h == 0 || tile.len() != cells * 3 || arena.len() != cells {
            return Err(SplatError::BadDim);
        }
        for px in tile.chunks_exact_mut(3) {
            px.copy_from_slice(&[16, 32, 48]);
        }
        for o in Self::sort_depth(&self.cooked)? {
            let at = usize::try_from(o).map_err(|_| SplatError::BadDim)? * 5;
            let qb = self.cooked.get(at..at + 5).ok_or(SplatError::BadDim)?;
            let q: [u8; 5] = qb.try_into().map_err(|_| SplatError::BadDim)?;
            let pz = unit_to_f32(q[2]) * 8.0 + 4.0;
            if pz <= 0.0 {
                continue;
            }
            let psx = self.fx * (unit_to_f32(q[0]) * 8.0 - 4.0) / pz + self.cx;
            let psy = self.fy * (unit_to_f32(q[1]) * 8.0 - 4.0) / pz + self.cy;
            let sc = f32::from((q[3] >> 4) & 15) / 15.0 * 0.9 + 0.1;
            let qz = f32::from(q[3] & 15) / 15.0 * 2.0 - 1.0;
            let qw = (1.0 - qz * qz).max(0.0).sqrt();
            let cov = Self::cov2d(sc, sc, qz, qw);
            let rad = cov[0].max(cov[2]).sqrt() + 1.0;
            let col = unit_to_f32(q[4]);
            let mut iu = 0usize;
            let mut yf = 0.5f32;
            for _ in 0..h {
                let dy = yf - psy;
                yf += 1.0;
                if dy * dy >= rad * rad * 2.0 {
                    iu += wu;
                    continue;
                }
                let mut xf = 0.5f32;
                for _ in 0..w {
                    if pz < arena[iu] {
                        let dx = xf - psx;
                        let a = (1.0 - (dx * dx + dy * dy) / (rad * rad * 2.0)) * 0.8;
                        if a > 0.0 {
                            for c in 0..3 {
                                let old = unit_to_f32(tile[iu * 3 + c]);
                                tile[iu * 3 + c] = unit_to_u8(old * (1.0 - a) + col * a);
                            }
                        }
                    }
                    iu += 1;
                    xf += 1.0;
                }
            }
        }
        Ok(engine_core::hash_bytes(tile))
    }
}
fn unit_to_u8(v: f32) -> u8 {
    if v.is_nan() || v <= 0.0 {
        return 0;
    }
    if v >= 1.0 {
        return 255;
    }
    u8::try_from(trunc_u32(v * 255.0 + 0.5).min(255)).unwrap_or(255)
}
fn unit_to_f32(b: u8) -> f32 {
    f32::from(b) / 255.0
}
fn trunc_u32(v: f32) -> u32 {
    if !(v > 0.0) {
        return 0;
    }
    let bits = v.to_bits();
    let exp = (bits >> 23) & 255;
    if exp < 127 {
        return 0;
    }
    if exp - 127 > 30 {
        return u32::MAX;
    }
    let mant = (bits & 0x7F_FFFF) | 0x80_0000;
    let shift = exp - 127;
    // One shift both ways; saturating keeps debug builds panic-free.
    (mant >> 23u32.saturating_sub(shift)) << shift.saturating_sub(23)
}
fn header_end(ply: &[u8]) -> Option<usize> {
    let tag = b"end_header\n";
    let at = ply.windows(tag.len()).position(|w| w == &tag[..])?;
    Some(at + tag.len())
}
fn read_f32_le(body: &[u8], at: usize) -> Option<f32> {
    let b = body.get(at..)?.get(..4)?;
    Some(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
fn cook_ply(ply: &[u8]) -> Result<Vec<u8>, SplatError> {
    let at = header_end(ply).ok_or(SplatError::BadPly)?;
    let body = ply.get(at..).ok_or(SplatError::BadPly)?;
    if body.is_empty() || body.len() % 52 != 0 {
        return Err(SplatError::BadPly);
    }
    let mut bytes = Vec::with_capacity(body.len() / 52 * 5);
    for base in (0..body.len()).step_by(52) {
        let g = |o: usize| read_f32_le(body, base + o).ok_or(SplatError::BadPly);
        let (px, py, pz) = (g(0)?, g(4)?, g(8)?);
        let (sx, qz, rr) = (g(12)?, g(32)?, g(40)?);
        let srb = (unit_to_u8((sx - 0.1) / 0.9) >> 4) << 4 | (unit_to_u8((qz + 1.0) / 2.0) >> 4);
        bytes.push(unit_to_u8((px + 4.0) / 8.0));
        bytes.push(unit_to_u8((py + 4.0) / 8.0));
        bytes.push(unit_to_u8((pz - 4.0) / 8.0));
        bytes.push(srb);
        bytes.push(unit_to_u8(rr * 0.5 + 0.5));
    }
    Ok(bytes)
}
