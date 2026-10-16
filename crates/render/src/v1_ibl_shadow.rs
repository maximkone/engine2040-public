//! v1 IBL + shadow map (R2): Poly Haven HDRI environment sampling with a
//! roughness prefilter stub, plus a single depth-map shadow path from the
//! directional light.
//!
//! Stays behind the `v1` feature with the rest of R1; consumers are R4
//! (cascades/probes) and GM1. Everything here is procedural CPU data tables —
//! no A3 assets needed — and pixels never enter any simulation hash.
//!
//! * IBL: [`load_hdri`] decodes a CC0 Radiance HDR file (read-only fixture in
//!   `content/incoming/`), [`sample`] bilinearly samples linear radiance for a
//!   direction, [`prefilter`] blurs with a fixed 9-tap kernel whose spread
//!   grows with roughness, [`env_average`] gives the mean texel (exposure
//!   calibration for R4).
//! * Shadow: [`bake_shadow`] rasterizes mesh depth in light space,
//!   [`shadow_factor`] compares with bias plus a fixed 4-tap PCF. Diagnostic
//!   views for R4 come later; this is the depth + compare contract only.

use std::f32::consts::PI;
use std::fmt;

use crate::v1::MeshData;

// ---------------------------------------------------------------------------
// HDRI environment map.
// ---------------------------------------------------------------------------

/// Largest accepted HDR byte size (the 1k Poly Haven fixture is ~1.6 MiB).
pub const MAX_HDRI_BYTES: usize = 32 << 20;
/// Largest accepted HDRI dimension; bounds allocation before decode.
pub const MAX_HDRI_DIM: u32 = 4096;

/// HDRI load failures: bounded diagnostics, never a panic on file data.
#[derive(Clone, PartialEq, Debug)]
pub enum HdriError {
    /// No bytes at all.
    Empty,
    /// Byte count exceeds [`MAX_HDRI_BYTES`].
    TooLarge {
        /// Offending byte count.
        bytes: usize,
    },
    /// The `image` HDR decoder refused the bytes; message carried.
    Decode(String),
    /// Decoded to zero width/height or zero pixels.
    ZeroSized,
    /// A decoded dimension exceeds [`MAX_HDRI_DIM`].
    DimTooLarge {
        /// Decoded width.
        width: u32,
        /// Decoded height.
        height: u32,
    },
    /// A decoded texel is NaN/infinite; texel index carried.
    NonFinite {
        /// Texel index holding the bad value.
        texel: usize,
    },
}

impl fmt::Display for HdriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "hdri has no bytes"),
            Self::TooLarge { bytes } => write!(f, "hdri {bytes} bytes exceeds limit"),
            Self::Decode(e) => write!(f, "hdri decode: {e}"),
            Self::ZeroSized => write!(f, "hdri decoded to zero size"),
            Self::DimTooLarge { width, height } => {
                write!(f, "hdri {width}x{height} exceeds limit")
            }
            Self::NonFinite { texel } => write!(f, "hdri texel {texel} is not finite"),
        }
    }
}

impl std::error::Error for HdriError {}

/// Direction-sample failures.
#[derive(Clone, PartialEq, Debug)]
pub enum SampleError {
    /// A direction component is NaN/infinite.
    NonFinite,
    /// Direction has ~zero length.
    DegenerateDirection,
    /// Roughness outside `0..=1`; value carried.
    RoughnessOutOfRange(f32),
}

impl fmt::Display for SampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => write!(f, "sample direction is not finite"),
            Self::DegenerateDirection => write!(f, "sample direction has ~zero length"),
            Self::RoughnessOutOfRange(v) => write!(f, "roughness {v} outside 0..=1"),
        }
    }
}

impl std::error::Error for SampleError {}

/// Linear-RGB HDR environment map in equirectangular layout, row-major.
#[derive(Clone, PartialEq, Debug)]
pub struct EnvMap {
    /// Texel columns.
    pub width: u32,
    /// Texel rows.
    pub height: u32,
    /// `width * height` linear radiance texels, components `>= 0`.
    pub pixels: Vec<[f32; 3]>,
}

/// Decode Radiance HDR bytes into an [`EnvMap`]. Negative components clamp
/// to zero (out-of-gamut encoder ringing, deterministic); NaN/infinite
/// components fail. Same bytes give bit-identical maps on every run.
pub fn load_hdri(bytes: &[u8]) -> Result<EnvMap, HdriError> {
    if bytes.is_empty() {
        return Err(HdriError::Empty);
    }
    if bytes.len() > MAX_HDRI_BYTES {
        return Err(HdriError::TooLarge { bytes: bytes.len() });
    }
    let img = image::load_from_memory(bytes).map_err(|e| HdriError::Decode(e.to_string()))?;
    let rgb = img.into_rgb32f();
    let (width, height) = (rgb.width(), rgb.height());
    if width == 0 || height == 0 {
        return Err(HdriError::ZeroSized);
    }
    if width > MAX_HDRI_DIM || height > MAX_HDRI_DIM {
        return Err(HdriError::DimTooLarge { width, height });
    }
    let mut pixels = Vec::with_capacity((width as usize) * (height as usize));
    for (i, p) in rgb.pixels().enumerate() {
        let [r, g, b] = p.0;
        if !r.is_finite() || !g.is_finite() || !b.is_finite() {
            return Err(HdriError::NonFinite { texel: i });
        }
        pixels.push([r.max(0.0), g.max(0.0), b.max(0.0)]);
    }
    Ok(EnvMap {
        width,
        height,
        pixels,
    })
}

fn check_dir(dir: [f32; 3]) -> Result<[f32; 3], SampleError> {
    if !dir.iter().all(|c| c.is_finite()) {
        return Err(SampleError::NonFinite);
    }
    let l2 = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2];
    if l2 < 1e-12 {
        return Err(SampleError::DegenerateDirection);
    }
    let l = l2.sqrt();
    Ok([dir[0] / l, dir[1] / l, dir[2] / l])
}

/// Direction to equirectangular UV: `u = 0.5 + atan2(x, z) / 2PI` (seam at
/// -Z, wraps), `v = acos(clamp(y)) / PI` (pole at +Y). Inverse of
/// [`uv_to_dir`]; R4 probes share this convention.
pub fn dir_to_uv(dir: [f32; 3]) -> [f32; 2] {
    let theta = dir[0].atan2(dir[2]);
    let u = 0.5 + theta / (2.0 * PI);
    let v = dir[1].clamp(-1.0, 1.0).acos() / PI;
    [u, v]
}

/// Equirectangular UV back to a unit direction (texel-center convention).
pub fn uv_to_dir(uv: [f32; 2]) -> [f32; 3] {
    let theta = (uv[0] - 0.5) * 2.0 * PI;
    let phi = uv[1].clamp(0.0, 1.0) * PI;
    let (sp, cp) = (phi.sin(), phi.cos());
    [sp * theta.sin(), cp, sp * theta.cos()]
}

/// Bilinearly sample linear radiance for `dir`. U wraps (seam-free
/// longitude), V clamps (pole rows duplicate). Deterministic per map.
pub fn sample(env: &EnvMap, dir: [f32; 3]) -> Result<[f32; 3], SampleError> {
    let d = check_dir(dir)?;
    let [u, v] = dir_to_uv(d);
    let w = env.width as f32;
    let h = env.height as f32;
    // Texel-center mapping: u in [0,1) spans columns 0..w.
    let x = u * w - 0.5;
    let y = (v * h - 0.5).clamp(0.0, h - 1.0);
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let at = |ix: i32, iy: i32| -> [f32; 3] {
        // Wrap longitude, clamp latitude: the seam and poles stay defined.
        let xx = ix.rem_euclid(env.width as i32) as usize;
        let yy = (iy.clamp(0, env.height as i32 - 1)) as usize;
        env.pixels[yy * env.width as usize + xx]
    };
    let a = at(x0, y0);
    let b = at(x0 + 1, y0);
    let c = at(x0, y0 + 1);
    let e = at(x0 + 1, y0 + 1);
    let mix = |p: [f32; 3], q: [f32; 3], t: f32| {
        [
            p[0] + (q[0] - p[0]) * t,
            p[1] + (q[1] - p[1]) * t,
            p[2] + (q[2] - p[2]) * t,
        ]
    };
    Ok(mix(mix(a, b, fx), mix(c, e, fx), fy))
}

/// Fixed 8-neighbor ring for the prefilter kernel (compass offsets, unit-ish
/// radius); with the center tap the kernel is 9 taps, equal-weighted.
pub const PREFILTER_RING: [[f32; 2]; 8] = [
    [1.0, 0.0],
    [1.0, 1.0],
    [0.0, 1.0],
    [-1.0, 1.0],
    [-1.0, 0.0],
    [-1.0, -1.0],
    [0.0, -1.0],
    [1.0, -1.0],
];
/// Tangent-plane spread of the ring at roughness 1 (~0.35 rad of blur).
pub const PREFILTER_SPREAD: f32 = 0.35;

/// Roughness-blurred radiance stub over the HDRI: roughness 0 returns the
/// direct [`sample`]; above 0 the 9-tap ring spreads on the tangent plane
/// proportionally to roughness and averages. Equal weights keep the
/// brightest-spot monotonic property the tests pin: blurring a global-max
/// neighborhood can only dilute it. R4 replaces the kernel with mip levels;
/// the (`dir`, `roughness`) signature stays.
pub fn prefilter(env: &EnvMap, dir: [f32; 3], roughness: f32) -> Result<[f32; 3], SampleError> {
    if !roughness.is_finite() || !(0.0..=1.0).contains(&roughness) {
        return Err(SampleError::RoughnessOutOfRange(roughness));
    }
    let d = check_dir(dir)?;
    if roughness <= 0.0 {
        return sample(env, d);
    }
    // Deterministic tangent frame: +Y helper unless looking along +-Y.
    let helper: [f32; 3] = if d[1].abs() > 0.99 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let mut t = cross(helper, d);
    let tl = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt().max(1e-12);
    t = [t[0] / tl, t[1] / tl, t[2] / tl];
    let b = cross(d, t);
    let norm = |v: [f32; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
        [v[0] / l, v[1] / l, v[2] / l]
    };
    let s = roughness * PREFILTER_SPREAD;
    let mut acc = sample(env, d)?;
    for [ox, oy] in PREFILTER_RING {
        let off = norm([
            d[0] + (t[0] * ox + b[0] * oy) * s,
            d[1] + (t[1] * ox + b[1] * oy) * s,
            d[2] + (t[2] * ox + b[2] * oy) * s,
        ]);
        let tap = sample(env, off)?;
        acc = [acc[0] + tap[0], acc[1] + tap[1], acc[2] + tap[2]];
    }
    Ok([acc[0] / 9.0, acc[1] / 9.0, acc[2] / 9.0])
}

/// Mean texel radiance (f64 accumulation, fixed order): exposure calibration
/// input for R4/GM1. Deterministic per map.
pub fn env_average(env: &EnvMap) -> [f32; 3] {
    let mut acc = [0.0f64; 3];
    for p in &env.pixels {
        acc[0] += f64::from(p[0]);
        acc[1] += f64::from(p[1]);
        acc[2] += f64::from(p[2]);
    }
    let n = env.pixels.len().max(1) as f64;
    [
        (acc[0] / n) as f32,
        (acc[1] / n) as f32,
        (acc[2] / n) as f32,
    ]
}

/// Luminance (Rec.709) helper for test assertions and R4 calibration.
pub fn luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

// ---------------------------------------------------------------------------
// Single shadow map: ortho depth from the directional light + bias + PCF.
// ---------------------------------------------------------------------------

/// Default shadow texel resolution (64x64 keeps the CPU bake test-fast).
pub const SHADOW_SIZE: u32 = 64;
/// Smallest accepted shadow resolution.
pub const MIN_SHADOW_SIZE: u32 = 8;
/// Largest accepted shadow resolution; bounds the depth buffer.
pub const MAX_SHADOW_SIZE: u32 = 512;
/// Depth bias in normalized depth units: the comparison is
/// `frag_depth - SHADOW_BIAS <= map_depth` for lit, so the caster's own
/// surface (equal depth) counts as lit — no acne — while a point strictly
/// behind the stored depth by more than the bias counts as shadowed.
pub const SHADOW_BIAS: f32 = 0.002;
/// Fixed 2x2 PCF tap offsets in texels; [`shadow_factor`] returns the lit
/// fraction over these taps (1.0 fully lit, 0.0 fully shadowed).
pub const SHADOW_PCF_TAPS: [[f32; 2]; 4] = [[-0.5, -0.5], [0.5, -0.5], [-0.5, 0.5], [0.5, 0.5]];
/// Ortho half-extent in meters: the light-space box spans
/// `-half..+half` in U/V and maps that same span to depth `0..1`.
pub const SHADOW_HALF_EXTENT: f32 = 4.0;

/// Shadow failures: typed, no panics on caller data.
#[derive(Clone, PartialEq, Debug)]
pub enum ShadowError {
    /// A vector input is NaN/infinite; which field carried.
    NonFinite(&'static str),
    /// Light direction has ~zero length.
    DegenerateDirection,
    /// Shadow size outside `MIN..=MAX_SHADOW_SIZE`.
    BadSize(u32),
    /// Mesh has no triangles to bake.
    EmptyMesh,
    /// Half-extent not finite and positive.
    BadExtent(f32),
}

impl fmt::Display for ShadowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(w) => write!(f, "shadow {w} is not finite"),
            Self::DegenerateDirection => write!(f, "shadow light direction has ~zero length"),
            Self::BadSize(s) => write!(f, "shadow size {s} outside bounds"),
            Self::EmptyMesh => write!(f, "shadow mesh has no triangles"),
            Self::BadExtent(v) => write!(f, "shadow half-extent {v} is not positive finite"),
        }
    }
}

impl std::error::Error for ShadowError {}

/// Baked ortho depth as seen from the directional light. `light_dir` points
/// from the surface TOWARD the light (R1 `DirectionalLight` convention);
/// `center` is the ortho box center in meters. Depths are normalized `0..1`
/// (nearest..farthest over `-half..+half` along the ray); untouched texels
/// stay 1.0 (far).
#[derive(Clone, PartialEq, Debug)]
pub struct ShadowMap {
    /// Texel resolution (square).
    pub size: u32,
    /// `size * size` depths, row-major.
    pub depths: Vec<f32>,
    /// Normalized direction toward the light.
    pub light_dir: [f32; 3],
    /// Ortho box center in meters.
    pub center: [f32; 3],
    /// Ortho half-extent in meters.
    pub half_extent: f32,
}

fn check_light_dir(light_dir: [f32; 3]) -> Result<[f32; 3], ShadowError> {
    if !light_dir.iter().all(|c| c.is_finite()) {
        return Err(ShadowError::NonFinite("light_dir"));
    }
    let l2 =
        light_dir[0] * light_dir[0] + light_dir[1] * light_dir[1] + light_dir[2] * light_dir[2];
    if l2 < 1e-12 {
        return Err(ShadowError::DegenerateDirection);
    }
    let l = l2.sqrt();
    Ok([light_dir[0] / l, light_dir[1] / l, light_dir[2] / l])
}

/// Orthonormal light frame shared by bake and sample: `fwd` is the ray
/// direction (from light into the scene), `t`/`u` span the shadow UV plane.
fn light_basis(light_dir: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let fwd = [-light_dir[0], -light_dir[1], -light_dir[2]];
    let helper: [f32; 3] = if fwd[1].abs() > 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let mut t = [
        helper[1] * fwd[2] - helper[2] * fwd[1],
        helper[2] * fwd[0] - helper[0] * fwd[2],
        helper[0] * fwd[1] - helper[1] * fwd[0],
    ];
    let tl = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt().max(1e-12);
    t = [t[0] / tl, t[1] / tl, t[2] / tl];
    let u = [
        fwd[1] * t[2] - fwd[2] * t[1],
        fwd[2] * t[0] - fwd[0] * t[2],
        fwd[0] * t[1] - fwd[1] * t[0],
    ];
    (t, u, fwd)
}

/// World point into shadow UV plus normalized depth. Returns `None` outside
/// the ortho box (callers treat that as lit: no occluder information).
pub fn light_space_uv(map: &ShadowMap, point: [f32; 3]) -> Option<[f32; 3]> {
    let (t, u, fwd) = light_basis(map.light_dir);
    let rel = [
        point[0] - map.center[0],
        point[1] - map.center[1],
        point[2] - map.center[2],
    ];
    let lx = rel[0] * t[0] + rel[1] * t[1] + rel[2] * t[2];
    let ly = rel[0] * u[0] + rel[1] * u[1] + rel[2] * u[2];
    let lz = rel[0] * fwd[0] + rel[1] * fwd[1] + rel[2] * fwd[2];
    let h = map.half_extent;
    if lx < -h || lx > h || ly < -h || ly > h || lz < -h || lz > h {
        return None;
    }
    Some([
        (lx / h) * 0.5 + 0.5,
        (ly / h) * 0.5 + 0.5,
        (lz / h) * 0.5 + 0.5,
    ])
}

/// Rasterize `mesh` depth from the directional light into a square depth map:
/// project triangles orthographically into the light box, keep the nearest
/// depth per texel. Fixed triangle order and float math make the bake
/// deterministic per input.
pub fn bake_shadow(
    mesh: &MeshData,
    light_dir: [f32; 3],
    center: [f32; 3],
    half_extent: f32,
    size: u32,
) -> Result<ShadowMap, ShadowError> {
    let dir = check_light_dir(light_dir)?;
    if !center.iter().all(|c| c.is_finite()) {
        return Err(ShadowError::NonFinite("center"));
    }
    if !half_extent.is_finite() || half_extent <= 0.0 {
        return Err(ShadowError::BadExtent(half_extent));
    }
    if !(MIN_SHADOW_SIZE..=MAX_SHADOW_SIZE).contains(&size) {
        return Err(ShadowError::BadSize(size));
    }
    if mesh.indices.len() < 3 {
        return Err(ShadowError::EmptyMesh);
    }
    let map = ShadowMap {
        size,
        depths: vec![1.0f32; (size as usize) * (size as usize)],
        light_dir: dir,
        center,
        half_extent,
    };
    let (t, u, fwd) = light_basis(dir);
    let proj = |p: [f32; 3]| -> [f32; 3] {
        let rel = [p[0] - center[0], p[1] - center[1], p[2] - center[2]];
        let lx = rel[0] * t[0] + rel[1] * t[1] + rel[2] * t[2];
        let ly = rel[0] * u[0] + rel[1] * u[1] + rel[2] * u[2];
        let lz = rel[0] * fwd[0] + rel[1] * fwd[1] + rel[2] * fwd[2];
        [
            (lx / half_extent) * 0.5 + 0.5,
            (ly / half_extent) * 0.5 + 0.5,
            (lz / half_extent) * 0.5 + 0.5,
        ]
    };
    let edge = |ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32| {
        (px - ax) * (by - ay) - (py - ay) * (bx - ax)
    };
    let mut out = map;
    let n = size as usize;
    for tri in mesh.indices.chunks_exact(3) {
        let a = proj(mesh.positions[tri[0] as usize]);
        let b = proj(mesh.positions[tri[1] as usize]);
        let c = proj(mesh.positions[tri[2] as usize]);
        // Ortho depth outside the box never occludes a sampled point (which
        // itself must be inside the box), so clamp to the box walls.
        let clamp01 = |v: f32| v.clamp(0.0, 1.0);
        let (ax, ay) = (clamp01(a[0]) * size as f32, clamp01(a[1]) * size as f32);
        let (bx, by) = (clamp01(b[0]) * size as f32, clamp01(b[1]) * size as f32);
        let (cx, cy) = (clamp01(c[0]) * size as f32, clamp01(c[1]) * size as f32);
        let area = edge(ax, ay, bx, by, cx, cy);
        if area.abs() < 1e-9 {
            continue;
        }
        let x0 = (ax.min(bx).min(cx).floor().max(0.0) as usize).min(n.saturating_sub(1));
        let x1 = (ax.max(bx).max(cx).ceil().max(0.0) as usize).min(n.saturating_sub(1));
        let y0 = (ay.min(by).min(cy).floor().max(0.0) as usize).min(n.saturating_sub(1));
        let y1 = (ay.max(by).max(cy).ceil().max(0.0) as usize).min(n.saturating_sub(1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let wa = edge(bx, by, cx, cy, px, py) / area;
                let wb = edge(cx, cy, ax, ay, px, py) / area;
                let wc = edge(ax, ay, bx, by, px, py) / area;
                if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                // Orthographic projection: affine depth interpolation.
                let z = wa * a[2] + wb * b[2] + wc * c[2];
                let z = z.clamp(0.0, 1.0);
                let idx = y * n + x;
                if z < out.depths[idx] {
                    out.depths[idx] = z;
                }
            }
        }
    }
    Ok(out)
}

/// Lit fraction at `point` (1.0 fully lit, 0.0 fully shadowed): each PCF tap
/// compares `frag_depth - SHADOW_BIAS <= stored_depth`; points outside the
/// ortho box are lit. Deterministic per map and point.
pub fn shadow_factor(map: &ShadowMap, point: [f32; 3]) -> Result<f32, ShadowError> {
    if !point.iter().all(|c| c.is_finite()) {
        return Err(ShadowError::NonFinite("point"));
    }
    let Some([u, v, depth]) = light_space_uv(map, point) else {
        return Ok(1.0);
    };
    let n = map.size as usize;
    let mut lit = 0u32;
    for [ox, oy] in SHADOW_PCF_TAPS {
        let sx = ((u * map.size as f32 + ox).floor() as i32).clamp(0, map.size as i32 - 1) as usize;
        let sy = ((v * map.size as f32 + oy).floor() as i32).clamp(0, map.size as i32 - 1) as usize;
        if depth - SHADOW_BIAS <= map.depths[sy * n + sx] {
            lit += 1;
        }
    }
    Ok(lit as f32 / SHADOW_PCF_TAPS.len() as f32)
}

// ---------------------------------------------------------------------------
// Tests: HDRI load + finite radiance, prefilter monotonic-ish, shadow bias
// direction, determinism, plus failure cases.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::cube;

    /// The Poly Haven CC0 studio HDRI, read-only fixture (never written).
    const HDRI_BYTES: &[u8] =
        include_bytes!("../../../content/arena/hdri/polyhaven-studio-small-09-1k.hdr");

    fn load_fixture() -> EnvMap {
        load_hdri(HDRI_BYTES).expect("studio HDRI fixture decodes")
    }

    #[test]
    fn hdri_loads_and_samples_finite_radiance() {
        let env = load_fixture();
        // Fixture shape: 1k equirect, nonzero, pixels match dimensions.
        assert!(env.width > 0 && env.height > 0, "hdri has nonzero size");
        assert_eq!(env.pixels.len(), env.width as usize * env.height as usize);
        assert!(env.width <= MAX_HDRI_DIM && env.height <= MAX_HDRI_DIM);
        let avg = env_average(&env);
        assert!(
            avg.iter().all(|c| c.is_finite() && *c >= 0.0),
            "mean radiance is finite/nonnegative, got {avg:?}"
        );
        assert!(
            luminance(avg) > 0.01,
            "studio hdri is not black, mean luminance {}",
            luminance(avg)
        );
        // Fixed directions plus poles/seam: every sample finite/nonnegative.
        let dirs = [
            [1.0, 0.0, 0.0],
            [-1.0, 0.2, 0.1],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
            [0.3, -0.5, 0.8],
        ];
        for d in dirs {
            let s = sample(&env, d).expect("fixed direction samples");
            assert!(
                s.iter().all(|c| c.is_finite() && *c >= 0.0),
                "sample at {d:?} is finite/nonnegative, got {s:?}"
            );
        }
    }

    #[test]
    fn roughness_prefilter_varies_monotonically_ish() {
        // Exact assertion (documented): at the brightest texel's direction,
        // the sharp sample strictly exceeds both blurs by a real margin
        // (blurring a global-max neighborhood dilutes it), and the two blurs
        // agree within 2% of sharp (a wider blur of an already-diluted
        // neighborhood plateaus). This is the "monotonically-ish" contract:
        // a big roughness-driven drop off the peak, then a stable plateau —
        // stepwise mid >= blur is NOT asserted because box-blur radii do not
        // nest pointwise. Precondition: the sharp spot exceeds the map mean
        // by 0.05, true for the studio softbox; without a bright spot there
        // is no gradient to vary over.
        let env = load_fixture();
        let avg = luminance(env_average(&env));
        let (mut best_i, mut best_l) = (0usize, f32::NEG_INFINITY);
        for (i, p) in env.pixels.iter().enumerate() {
            let l = luminance(*p);
            if l > best_l {
                best_l = l;
                best_i = i;
            }
        }
        let w = env.width as usize;
        let u = (best_i % w) as f32 / env.width as f32;
        let v = (best_i / w) as f32 / env.height as f32;
        let dir = uv_to_dir([u, v]);
        let sharp = luminance(sample(&env, dir).expect("bright spot samples"));
        assert!(
            sharp > avg + 0.05,
            "fixture needs a bright spot for this assertion (sharp {sharp}, avg {avg})"
        );
        let mid = luminance(prefilter(&env, dir, 0.5).expect("mid roughness prefilters"));
        let blur = luminance(prefilter(&env, dir, 1.0).expect("full roughness prefilters"));
        assert!(
            sharp > mid + 1.0 && sharp > blur + 1.0,
            "roughness dilutes the bright spot: sharp {sharp}, mid {mid}, blur {blur}"
        );
        assert!(
            (mid - blur).abs() <= 0.02 * sharp,
            "blurs plateau within 2% of sharp: sharp {sharp}, mid {mid}, blur {blur}"
        );
        // Roughness 0 is the direct sample, exactly.
        let r0 = prefilter(&env, dir, 0.0).expect("roughness 0 prefilters");
        assert_eq!(r0, sample(&env, dir).expect("direct sample"));
    }

    #[test]
    fn shadow_depth_bias_direction_is_correct() {
        // Overhead light (+Y toward the light) over the unit cube: a point
        // 0.1 m above the top face is lit (1.0), a point 0.1 m below it —
        // inside the cube, behind the stored top-face depth — is shadowed
        // (0.0). This pins the bias direction: equal depth counts as lit.
        let mesh = cube();
        let map = bake_shadow(
            &mesh,
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            SHADOW_HALF_EXTENT,
            SHADOW_SIZE,
        )
        .expect("cube shadow bakes");
        assert_eq!(
            map.depths.len(),
            SHADOW_SIZE as usize * SHADOW_SIZE as usize
        );
        assert!(
            map.depths.iter().any(|d| *d < 1.0),
            "the cube must write depth texels"
        );
        let lit = shadow_factor(&map, [0.0, 0.6, 0.0]).expect("above-face samples");
        let dark = shadow_factor(&map, [0.0, 0.4, 0.0]).expect("below-face samples");
        assert_eq!(lit, 1.0, "point above the caster is lit");
        assert_eq!(dark, 0.0, "point behind the stored depth is shadowed");
        // Outside the ortho box: lit (no occluder information, never dark).
        let far = shadow_factor(&map, [100.0, 0.0, 0.0]).expect("out-of-box samples");
        assert_eq!(far, 1.0, "points outside the map are lit");
    }

    #[test]
    fn ibl_and_shadow_are_deterministic() {
        // Same complete inputs twice give identical outputs.
        let a = load_fixture();
        let b = load_fixture();
        assert_eq!(a, b, "hdri decode is bit-identical twice");
        let dir = [0.4, 0.35, 0.8];
        assert_eq!(
            sample(&a, dir),
            sample(&b, dir),
            "same direction samples identically"
        );
        assert_eq!(
            prefilter(&a, dir, 0.6),
            prefilter(&b, dir, 0.6),
            "same prefilter inputs match identically"
        );
        assert_eq!(
            env_average(&a),
            env_average(&b),
            "map averages match identically"
        );
        let mesh = cube();
        let m1 = bake_shadow(
            &mesh,
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            SHADOW_HALF_EXTENT,
            SHADOW_SIZE,
        )
        .expect("first bake works");
        let m2 = bake_shadow(
            &mesh,
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            SHADOW_HALF_EXTENT,
            SHADOW_SIZE,
        )
        .expect("second bake works");
        assert_eq!(m1, m2, "shadow bake is bit-identical twice");
        assert_eq!(
            shadow_factor(&m1, [0.0, 0.6, 0.0]),
            shadow_factor(&m2, [0.0, 0.6, 0.0]),
            "shadow samples match identically"
        );
    }

    #[test]
    fn invalid_inputs_fail_loudly() {
        assert_eq!(load_hdri(&[]), Err(HdriError::Empty));
        assert!(matches!(load_hdri(&[0u8; 8]), Err(HdriError::Decode(_))));
        assert!(matches!(
            load_hdri(b"definitely not an image at all ........"),
            Err(HdriError::Decode(_))
        ));
        assert_eq!(
            load_hdri(&vec![0u8; MAX_HDRI_BYTES + 1]),
            Err(HdriError::TooLarge {
                bytes: MAX_HDRI_BYTES + 1
            })
        );
        let env = load_fixture();
        assert_eq!(
            sample(&env, [f32::NAN, 0.0, 0.0]),
            Err(SampleError::NonFinite)
        );
        assert_eq!(
            sample(&env, [0.0, 0.0, 0.0]),
            Err(SampleError::DegenerateDirection)
        );
        assert_eq!(
            prefilter(&env, [0.0, 1.0, 0.0], 2.0),
            Err(SampleError::RoughnessOutOfRange(2.0))
        );
        assert!(matches!(
            prefilter(&env, [0.0, 1.0, 0.0], f32::NAN),
            Err(SampleError::RoughnessOutOfRange(v)) if v.is_nan()
        ));
        let mesh = cube();
        assert_eq!(
            bake_shadow(
                &mesh,
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0],
                SHADOW_HALF_EXTENT,
                SHADOW_SIZE
            ),
            Err(ShadowError::DegenerateDirection)
        );
        assert_eq!(
            bake_shadow(
                &mesh,
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0],
                SHADOW_HALF_EXTENT,
                4
            ),
            Err(ShadowError::BadSize(4))
        );
        assert_eq!(
            bake_shadow(&mesh, [0.0, 1.0, 0.0], [0.0, 0.0, 0.0], -1.0, SHADOW_SIZE),
            Err(ShadowError::BadExtent(-1.0))
        );
        let map = bake_shadow(
            &mesh,
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            SHADOW_HALF_EXTENT,
            SHADOW_SIZE,
        )
        .expect("cube shadow bakes");
        assert_eq!(
            shadow_factor(&map, [f32::INFINITY, 0.0, 0.0]),
            Err(ShadowError::NonFinite("point"))
        );
    }
}
