//! v1 V2 lighting (R4): cascaded shadows, baked probe GI, bloom factors, TAA
//! jitter and color grading — all CPU-side data tables and pure functions
//! behind the `v1` feature, so the headless default stays std-only.
//!
//! Consumer: R6/PA1 (temporal upscaling, performance autopilot), GM1 (game
//! frame assembly) and Q1 (visual reference captures). Data owner:
//! `engine-render`; nothing here enters any simulation hash. Units: meters,
//! linear radiance, view depth in meters, NDC/pixel offsets for jitter.
//! Determinism scope: fixed triangle/order, fixed tap orders, f64
//! accumulation for means; same complete inputs give bit-identical outputs.
//! Errors are typed; invalid caller data never panics.
//!
//! Builds on the R1 v1 tonemap path (`aces_fitted`, `linear_to_srgb`) and the
//! R2 ortho shadow path (`bake_shadow`, `light_space_uv`, `SHADOW_PCF_TAPS`)
//! without changing either file: per-cascade bias needs its own compare, so
//! the cascade sample reuses the R2 UV/PCF tables with a per-level bias.
//! GPU application (projection jitter, grade uniforms, bloom add) is consumed
//! by `v1_gpu`; the wgpu pass wiring itself lands with the temporal upscaler.
//!
//! Provenance: cascade practical splits, Halton-2/3 jitter, trilinear probe
//! lookup, box-filter bloom mips and exposure/contrast/saturation grading are
//! textbook formulations implemented directly (no upstream copy), so they
//! carry no `PORT()` marker. The tonemap they feed stays Bevy-ported in
//! `v1.rs`.

use std::fmt;

use crate::v1::{aces_fitted, linear_to_srgb, MAX_CAPTURE_DIM};
use crate::v1_ibl_shadow::{
    bake_shadow, light_space_uv, luminance, prefilter, EnvMap, SampleError, ShadowError, ShadowMap,
    MAX_SHADOW_SIZE, MIN_SHADOW_SIZE, SHADOW_PCF_TAPS,
};

// ---------------------------------------------------------------------------
// Cascaded shadows: 3 cascades over the R2 ortho shadow path.
// ---------------------------------------------------------------------------

/// Cascade count over the camera range.
pub const CASCADE_COUNT: usize = 3;
/// Practical-split blend: half logarithmic (depth precision), half uniform
/// (even texel use). Fixed so every run splits identically.
pub const CASCADE_LAMBDA: f32 = 0.5;

/// Cascade failures: typed, no panics on caller data.
#[derive(Clone, PartialEq, Debug)]
pub enum CascadeError {
    /// A scalar input is NaN/infinite; which field carried.
    NonFinite(&'static str),
    /// Range is not `0 < near < far` and finite.
    BadRange { near: f32, far: f32 },
    /// A half-extent is not finite and positive; value carried.
    BadExtent(f32),
    /// A bias is negative or non-finite; value carried.
    BadBias(f32),
    /// Shadow size outside `MIN..=MAX_SHADOW_SIZE`.
    BadSize(u32),
    /// The R2 bake refused the input.
    Shadow(ShadowError),
}

impl fmt::Display for CascadeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(w) => write!(f, "cascade {w} is not finite"),
            Self::BadRange { near, far } => {
                write!(
                    f,
                    "cascade range near={near} far={far} is not 0 < near < far"
                )
            }
            Self::BadExtent(v) => write!(f, "cascade half-extent {v} is not positive finite"),
            Self::BadBias(v) => write!(f, "cascade bias {v} is not finite non-negative"),
            Self::BadSize(s) => write!(f, "cascade size {s} outside bounds"),
            Self::Shadow(e) => write!(f, "cascade shadow: {e}"),
        }
    }
}

impl std::error::Error for CascadeError {}

impl From<ShadowError> for CascadeError {
    fn from(e: ShadowError) -> Self {
        Self::Shadow(e)
    }
}

/// Practical split distances over `[near, far]`: `splits[0] == near`,
/// `splits[3] == far`, strictly increasing, so the three `[splits[i],
/// splits[i+1])` intervals tile the range with no gaps and no overlaps.
/// Each interior split blends the logarithmic and uniform placements with
/// [`CASCADE_LAMBDA`]; the endpoints are exact copies of the inputs.
pub fn practical_splits(near: f32, far: f32) -> Result<[f32; 4], CascadeError> {
    if !near.is_finite() || !far.is_finite() {
        return Err(CascadeError::NonFinite("range"));
    }
    if near <= 0.0 || far <= near {
        return Err(CascadeError::BadRange { near, far });
    }
    let mut splits = [near, 0.0, 0.0, far];
    for (i, slot) in splits.iter_mut().enumerate().take(CASCADE_COUNT).skip(1) {
        *slot = crate::shadow_cluster::practical_split(
            near,
            far,
            i as f32 / CASCADE_COUNT as f32,
            CASCADE_LAMBDA,
        );
    }
    Ok(splits)
}

/// One cascade: its depth interval plus its own ortho shadow map. `bias` is
/// per level because distant cascades span more meters per texel and need a
/// larger depth tolerance than the near cascade.
#[derive(Clone, PartialEq, Debug)]
pub struct CascadeLevel {
    /// Interval start in meters (view depth).
    pub near: f32,
    /// Interval end in meters (view depth).
    pub far: f32,
    /// Ortho half-extent in meters for this level's bake.
    pub half_extent: f32,
    /// Depth bias in normalized depth units for this level's compare.
    pub bias: f32,
    /// Baked depth; `None` until [`CascadeSet::bake_all`] runs.
    pub map: Option<ShadowMap>,
}

/// Three cascades tiling the camera `[near, far]` range.
#[derive(Clone, PartialEq, Debug)]
pub struct CascadeSet {
    /// Camera near in meters.
    pub near: f32,
    /// Camera far in meters.
    pub far: f32,
    /// One level per depth interval, near to far.
    pub levels: [CascadeLevel; 3],
}

impl CascadeSet {
    /// Build from the camera range plus per-level half-extents, biases and a
    /// shared shadow resolution. Splits come from [`practical_splits`], so
    /// identical inputs give identical intervals on every run.
    pub fn new(
        near: f32,
        far: f32,
        half_extents: [f32; 3],
        biases: [f32; 3],
        size: u32,
    ) -> Result<Self, CascadeError> {
        let splits = practical_splits(near, far)?;
        for h in half_extents {
            if !h.is_finite() || h <= 0.0 {
                return Err(CascadeError::BadExtent(h));
            }
        }
        for b in biases {
            if !b.is_finite() || b < 0.0 {
                return Err(CascadeError::BadBias(b));
            }
        }
        if !(MIN_SHADOW_SIZE..=MAX_SHADOW_SIZE).contains(&size) {
            return Err(CascadeError::BadSize(size));
        }
        let levels = [
            CascadeLevel {
                near: splits[0],
                far: splits[1],
                half_extent: half_extents[0],
                bias: biases[0],
                map: None,
            },
            CascadeLevel {
                near: splits[1],
                far: splits[2],
                half_extent: half_extents[1],
                bias: biases[1],
                map: None,
            },
            CascadeLevel {
                near: splits[2],
                far: splits[3],
                half_extent: half_extents[2],
                bias: biases[2],
                map: None,
            },
        ];
        Ok(Self { near, far, levels })
    }

    /// Cascade for `view_depth`: the first level whose `far` exceeds the
    /// depth (half-open intervals `[near_i, far_i)`), else the last level.
    /// Non-finite depths compare false everywhere and land in the last
    /// cascade; callers needing strictness validate first.
    pub fn cascade_index(&self, view_depth: f32) -> usize {
        for (i, level) in self.levels.iter().enumerate() {
            if view_depth < level.far {
                return i;
            }
        }
        CASCADE_COUNT - 1
    }

    /// Bake every level's ortho depth over the R2 shadow path with its own
    /// half-extent. Fixed triangle order per level keeps the bake
    /// deterministic per input.
    pub fn bake_all(
        &mut self,
        mesh: &crate::v1::MeshData,
        light_dir: [f32; 3],
        center: [f32; 3],
        size: u32,
    ) -> Result<(), CascadeError> {
        if !(MIN_SHADOW_SIZE..=MAX_SHADOW_SIZE).contains(&size) {
            return Err(CascadeError::BadSize(size));
        }
        for level in &mut self.levels {
            let map = bake_shadow(mesh, light_dir, center, level.half_extent, size)?;
            level.map = Some(map);
        }
        Ok(())
    }

    /// Lit fraction at `point` seen from `view_depth` (1.0 fully lit, 0.0
    /// fully shadowed): selects the cascade, then compares with that level's
    /// bias. Unbaked levels and points outside the ortho box are lit (no
    /// occluder information, never dark — the R2 convention).
    pub fn shadow_factor(&self, view_depth: f32, point: [f32; 3]) -> Result<f32, ShadowError> {
        cascade_factor(&self.levels[self.cascade_index(view_depth)], point)
    }
}

/// Lit fraction for one cascade level with its own bias: each R2 PCF tap
/// compares `frag_depth - bias <= stored_depth`. Same tables as R2
/// (`light_space_uv`, `SHADOW_PCF_TAPS`), per-level tolerance.
pub fn cascade_factor(level: &CascadeLevel, point: [f32; 3]) -> Result<f32, ShadowError> {
    let Some(map) = &level.map else {
        return Ok(1.0);
    };
    if !point.iter().all(|c| c.is_finite()) {
        return Err(ShadowError::NonFinite("point"));
    }
    if !level.bias.is_finite() || level.bias < 0.0 {
        return Err(ShadowError::NonFinite("bias"));
    }
    let Some([u, v, depth]) = light_space_uv(map, point) else {
        return Ok(1.0);
    };
    let n = map.size as usize;
    let mut lit = 0u32;
    for [ox, oy] in SHADOW_PCF_TAPS {
        let sx = ((u * map.size as f32 + ox).floor() as i32).clamp(0, map.size as i32 - 1) as usize;
        let sy = ((v * map.size as f32 + oy).floor() as i32).clamp(0, map.size as i32 - 1) as usize;
        if depth - level.bias <= map.depths[sy * n + sx] {
            lit += 1;
        }
    }
    Ok(lit as f32 / SHADOW_PCF_TAPS.len() as f32)
}

// ---------------------------------------------------------------------------
// Probe grid: baked irradiance samples as data tables with trilinear lookup.
// ---------------------------------------------------------------------------

/// Largest accepted probe count per axis; bounds `nx * ny * nz` before
/// allocation.
pub const MAX_PROBE_AXIS: u32 = 64;
/// Largest accepted total probe count.
pub const MAX_PROBES: usize = 1 << 15;

/// Probe failures: typed, no panics on caller data.
#[derive(Clone, PartialEq, Debug)]
pub enum ProbeError {
    /// A scalar input is NaN/infinite; which field carried.
    NonFinite(&'static str),
    /// A dimension is 0 or exceeds [`MAX_PROBE_AXIS`]; dims carried.
    BadDims([u32; 3]),
    /// A spacing component is not finite and positive; value carried.
    BadSpacing([f32; 3]),
    /// The grid exceeds [`MAX_PROBES`]; count carried.
    TooManyProbes { count: usize },
    /// Sample table length differs from `nx * ny * nz`.
    LengthMismatch { expected: usize, got: usize },
    /// Sampling a direction inside the environment bake failed.
    Sample(SampleError),
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(w) => write!(f, "probe {w} is not finite"),
            Self::BadDims(d) => write!(f, "probe dims {d:?} outside 1..=MAX_PROBE_AXIS"),
            Self::BadSpacing(s) => write!(f, "probe spacing {s:?} is not positive finite"),
            Self::TooManyProbes { count } => write!(f, "probe count {count} exceeds limit"),
            Self::LengthMismatch { expected, got } => {
                write!(f, "probe samples {got} differ from grid {expected}")
            }
            Self::Sample(e) => write!(f, "probe env sample: {e}"),
        }
    }
}

impl std::error::Error for ProbeError {}

/// Baked irradiance grid: `nx * ny * nz` linear-RGB samples in fixed
/// `(iz * ny + iy) * nx + ix` order. Probe `(ix, iy, iz)` sits at
/// `origin + [ix * sx, iy * sy, iz * sz]` in meters.
#[derive(Clone, PartialEq, Debug)]
pub struct ProbeGrid {
    /// Grid corner in meters (probe `(0, 0, 0)`).
    pub origin: [f32; 3],
    /// Meters between adjacent probes per axis.
    pub spacing: [f32; 3],
    /// Probe counts per axis.
    pub dims: [u32; 3],
    /// `nx * ny * nz` baked irradiance samples, components `>= 0`.
    pub samples: Vec<[f32; 3]>,
}

fn check_probe_shape(
    origin: [f32; 3],
    spacing: [f32; 3],
    dims: [u32; 3],
) -> Result<usize, ProbeError> {
    if !origin.iter().all(|c| c.is_finite()) {
        return Err(ProbeError::NonFinite("origin"));
    }
    if !spacing.iter().all(|c| c.is_finite()) {
        return Err(ProbeError::NonFinite("spacing"));
    }
    if spacing.iter().any(|c| *c <= 0.0) {
        return Err(ProbeError::BadSpacing(spacing));
    }
    if dims.iter().any(|d| *d == 0 || *d > MAX_PROBE_AXIS) {
        return Err(ProbeError::BadDims(dims));
    }
    let count = dims[0] as usize * dims[1] as usize * dims[2] as usize;
    if count > MAX_PROBES {
        return Err(ProbeError::TooManyProbes { count });
    }
    Ok(count)
}

impl ProbeGrid {
    /// Bake one irradiance sample per probe from `bake(pos)`: positions run
    /// the fixed index order, so the same function gives identical tables.
    pub fn bake(
        origin: [f32; 3],
        spacing: [f32; 3],
        dims: [u32; 3],
        bake: impl Fn([f32; 3]) -> [f32; 3],
    ) -> Result<Self, ProbeError> {
        let count = check_probe_shape(origin, spacing, dims)?;
        let mut samples = Vec::with_capacity(count);
        for iz in 0..dims[2] {
            for iy in 0..dims[1] {
                for ix in 0..dims[0] {
                    let pos = [
                        origin[0] + ix as f32 * spacing[0],
                        origin[1] + iy as f32 * spacing[1],
                        origin[2] + iz as f32 * spacing[2],
                    ];
                    let s = bake(pos);
                    if !s.iter().all(|c| c.is_finite() && *c >= 0.0) {
                        return Err(ProbeError::NonFinite("samples"));
                    }
                    samples.push(s);
                }
            }
        }
        Ok(Self {
            origin,
            spacing,
            dims,
            samples,
        })
    }

    /// Bake irradiance from an HDRI environment: each probe averages the
    /// fully-rough prefilter over the six axis directions (fixed order, f64
    /// accumulation). Position-independent for a pure environment map, but
    /// the table shape is what R4 consumers (and later spatially-varying
    /// bakes) read through [`sample_trilinear`].
    pub fn bake_from_env(
        env: &EnvMap,
        origin: [f32; 3],
        spacing: [f32; 3],
        dims: [u32; 3],
    ) -> Result<Self, ProbeError> {
        const AXES: [[f32; 3]; 6] = [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ];
        let count = check_probe_shape(origin, spacing, dims)?;
        let mut first: Option<[f32; 3]> = None;
        let mut samples = Vec::with_capacity(count);
        for _ in 0..count {
            let mut acc = [0.0f64; 3];
            for dir in AXES {
                let tap = prefilter(env, dir, 1.0).map_err(ProbeError::Sample)?;
                acc[0] += f64::from(tap[0]);
                acc[1] += f64::from(tap[1]);
                acc[2] += f64::from(tap[2]);
            }
            let mean = [
                (acc[0] / AXES.len() as f64) as f32,
                (acc[1] / AXES.len() as f64) as f32,
                (acc[2] / AXES.len() as f64) as f32,
            ];
            // Every probe shares the environment mean: bake once, reuse the
            // bit-identical value so position count never perturbs rounding.
            if first.is_none() {
                first = Some(mean);
            }
            samples.push(first.unwrap_or(mean));
        }
        Ok(Self {
            origin,
            spacing,
            dims,
            samples,
        })
    }

    /// Probe position for index `(ix, iy, iz)` in meters.
    pub fn probe_pos(&self, ix: u32, iy: u32, iz: u32) -> [f32; 3] {
        [
            self.origin[0] + ix as f32 * self.spacing[0],
            self.origin[1] + iy as f32 * self.spacing[1],
            self.origin[2] + iz as f32 * self.spacing[2],
        ]
    }
}

/// Trilinear irradiance lookup: blends the eight surrounding probes by the
/// fractional position. Positions outside the grid clamp to the edge (texture
/// convention, documented); only non-finite positions fail. Deterministic
/// per grid and position.
pub fn sample_trilinear(grid: &ProbeGrid, pos: [f32; 3]) -> Result<[f32; 3], ProbeError> {
    if !pos.iter().all(|c| c.is_finite()) {
        return Err(ProbeError::NonFinite("position"));
    }
    if grid.samples.len() != grid.dims[0] as usize * grid.dims[1] as usize * grid.dims[2] as usize {
        return Err(ProbeError::LengthMismatch {
            expected: grid.dims[0] as usize * grid.dims[1] as usize * grid.dims[2] as usize,
            got: grid.samples.len(),
        });
    }
    // Per-axis lower index + fraction, clamped into the grid.
    let mut idx = [0usize; 3];
    let mut frac = [0.0f32; 3];
    for a in 0..3 {
        let n = grid.dims[a] as usize;
        let local = (pos[a] - grid.origin[a]) / grid.spacing[a];
        let clamped = local.clamp(0.0, (n - 1) as f32);
        if n == 1 {
            idx[a] = 0;
            frac[a] = 0.0;
        } else {
            let i0 = (clamped.floor() as usize).min(n - 2);
            idx[a] = i0;
            frac[a] = (clamped - i0 as f32).clamp(0.0, 1.0);
        }
    }
    let at = |ix: usize, iy: usize, iz: usize| -> [f32; 3] {
        let nx = grid.dims[0] as usize;
        let ny = grid.dims[1] as usize;
        grid.samples[(iz * ny + iy) * nx + ix]
    };
    let mix = |p: [f32; 3], q: [f32; 3], t: f32| {
        [
            p[0] + (q[0] - p[0]) * t,
            p[1] + (q[1] - p[1]) * t,
            p[2] + (q[2] - p[2]) * t,
        ]
    };
    let (ix, iy, iz) = (idx[0], idx[1], idx[2]);
    let c000 = at(ix, iy, iz);
    let c100 = at((ix + 1).min(grid.dims[0] as usize - 1), iy, iz);
    let c010 = at(ix, (iy + 1).min(grid.dims[1] as usize - 1), iz);
    let c110 = at(
        (ix + 1).min(grid.dims[0] as usize - 1),
        (iy + 1).min(grid.dims[1] as usize - 1),
        iz,
    );
    let c001 = at(ix, iy, (iz + 1).min(grid.dims[2] as usize - 1));
    let c101 = at(
        (ix + 1).min(grid.dims[0] as usize - 1),
        iy,
        (iz + 1).min(grid.dims[2] as usize - 1),
    );
    let c011 = at(
        ix,
        (iy + 1).min(grid.dims[1] as usize - 1),
        (iz + 1).min(grid.dims[2] as usize - 1),
    );
    let c111 = at(
        (ix + 1).min(grid.dims[0] as usize - 1),
        (iy + 1).min(grid.dims[1] as usize - 1),
        (iz + 1).min(grid.dims[2] as usize - 1),
    );
    let x00 = mix(c000, c100, frac[0]);
    let x10 = mix(c010, c110, frac[0]);
    let x01 = mix(c001, c101, frac[0]);
    let x11 = mix(c011, c111, frac[0]);
    let y0 = mix(x00, x10, frac[1]);
    let y1 = mix(x01, x11, frac[1]);
    Ok(mix(y0, y1, frac[2]))
}

// ---------------------------------------------------------------------------
// Bloom: threshold + downsample/upsample factors as pure CPU functions.
// ---------------------------------------------------------------------------

/// Most mip levels a chain holds; bounds allocation before building.
pub const MAX_BLOOM_LEVELS: usize = 6;

/// Bloom failures: typed, no panics on caller data.
#[derive(Clone, PartialEq, Debug)]
pub enum BloomError {
    /// A scalar input is NaN/infinite; which field carried.
    NonFinite(&'static str),
    /// Dimensions are 0 or exceed [`MAX_CAPTURE_DIM`].
    BadDims { width: u32, height: u32 },
    /// Pixel count differs from `width * height`.
    LengthMismatch { expected: usize, got: usize },
    /// Threshold is negative or non-finite; value carried.
    BadThreshold(f32),
    /// Requested level count is 0; value carried.
    BadLevelCount(usize),
}

impl fmt::Display for BloomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(w) => write!(f, "bloom {w} is not finite"),
            Self::BadDims { width, height } => {
                write!(f, "bloom {width}x{height} outside bounds")
            }
            Self::LengthMismatch { expected, got } => {
                write!(f, "bloom pixels {got} differ from dims {expected}")
            }
            Self::BadThreshold(v) => write!(f, "bloom threshold {v} is not finite non-negative"),
            Self::BadLevelCount(n) => write!(f, "bloom level count {n} is zero"),
        }
    }
}

impl std::error::Error for BloomError {}

fn check_bloom_image(pixels: &[[f32; 3]], width: u32, height: u32) -> Result<(), BloomError> {
    if width == 0 || height == 0 || width > MAX_CAPTURE_DIM || height > MAX_CAPTURE_DIM {
        return Err(BloomError::BadDims { width, height });
    }
    if pixels.len() != width as usize * height as usize {
        return Err(BloomError::LengthMismatch {
            expected: width as usize * height as usize,
            got: pixels.len(),
        });
    }
    for p in pixels {
        if !p.iter().all(|c| c.is_finite()) {
            return Err(BloomError::NonFinite("pixels"));
        }
    }
    Ok(())
}

/// Bright pass: keeps texels whose Rec.709 luminance exceeds `threshold`,
/// drops the rest to black. Only removes energy (`sum(out) <= sum(in)`;
/// pinned by test), never creates it.
pub fn bright_pass(
    pixels: &[[f32; 3]],
    width: u32,
    height: u32,
    threshold: f32,
) -> Result<Vec<[f32; 3]>, BloomError> {
    check_bloom_image(pixels, width, height)?;
    if !threshold.is_finite() || threshold < 0.0 {
        return Err(BloomError::BadThreshold(threshold));
    }
    Ok(pixels
        .iter()
        .map(|p| {
            if luminance(*p) > threshold {
                *p
            } else {
                [0.0, 0.0, 0.0]
            }
        })
        .collect())
}

/// 2x2 box downsample (clamp-to-edge for odd dimensions): each destination
/// texel averages up to four source texels. Box averaging conserves energy
/// directionally — `sum(dst) * 4 == sum(src)` for even dimensions (pinned by
/// test within float rounding) and `sum(dst) <= sum(src)` always — because
/// every source texel contributes its full value exactly once per level.
pub fn downsample_2x2(
    pixels: &[[f32; 3]],
    width: u32,
    height: u32,
) -> Result<(Vec<[f32; 3]>, u32, u32), BloomError> {
    check_bloom_image(pixels, width, height)?;
    let dw = (width / 2).max(1);
    let dh = (height / 2).max(1);
    let w = width as usize;
    let h = height as usize;
    let mut out = Vec::with_capacity(dw as usize * dh as usize);
    for y in 0..dh as usize {
        for x in 0..dw as usize {
            let mut acc = [0.0f64; 3];
            let mut n = 0u32;
            for oy in 0..2 {
                for ox in 0..2 {
                    let sx = (x * 2 + ox).min(w - 1);
                    let sy = (y * 2 + oy).min(h - 1);
                    let p = pixels[sy * w + sx];
                    acc[0] += f64::from(p[0]);
                    acc[1] += f64::from(p[1]);
                    acc[2] += f64::from(p[2]);
                    n += 1;
                }
            }
            out.push([
                (acc[0] / f64::from(n)) as f32,
                (acc[1] / f64::from(n)) as f32,
                (acc[2] / f64::from(n)) as f32,
            ]);
        }
    }
    Ok((out, dw, dh))
}

/// Nearest-neighbor upsample of `detail` to the base size, added onto `base`:
/// the composite pass `v1_gpu` evaluates per level. Addition only moves
/// energy upward from already-conserved mips; it never invents detail.
pub fn upsample_add(
    base: &[[f32; 3]],
    base_w: u32,
    base_h: u32,
    detail: &[[f32; 3]],
    detail_w: u32,
    detail_h: u32,
) -> Result<Vec<[f32; 3]>, BloomError> {
    check_bloom_image(base, base_w, base_h)?;
    check_bloom_image(detail, detail_w, detail_h)?;
    let bw = base_w as usize;
    let dw = detail_w as usize;
    let dh = detail_h as usize;
    let mut out = Vec::with_capacity(base.len());
    for y in 0..base_h as usize {
        for x in 0..bw {
            let sx = (x * dw / bw).min(dw - 1);
            let sy = (y * dh / (base_h as usize)).min(dh - 1);
            let b = base[y * bw + x];
            let d = detail[sy * dw + sx];
            out.push([b[0] + d[0], b[1] + d[1], b[2] + d[2]]);
        }
    }
    Ok(out)
}

/// One bloom mip level: full-res bright pass at index 0, then each level
/// half the resolution of the previous one.
#[derive(Clone, PartialEq, Debug)]
pub struct BloomMip {
    /// Level width.
    pub width: u32,
    /// Level height.
    pub height: u32,
    /// `width * height` linear-RGB texels.
    pub data: Vec<[f32; 3]>,
}

/// CPU-side bloom chain: thresholded full-res image plus its downsampled
/// mips. `v1_gpu` composites with [`upsample_add`] and adds the result to
/// the frame before grading.
#[derive(Clone, PartialEq, Debug)]
pub struct BloomMipChain {
    /// Luminance threshold used by the bright pass.
    pub threshold: f32,
    /// Mips from full resolution down; at least one level always.
    pub levels: Vec<BloomMip>,
}

impl BloomMipChain {
    /// Build the chain: bright-pass the source, then downsample until the
    /// smaller side drops below 2 texels or `max_levels` is reached
    /// (`max_levels` caps at [`MAX_BLOOM_LEVELS`]).
    pub fn build(
        pixels: &[[f32; 3]],
        width: u32,
        height: u32,
        threshold: f32,
        max_levels: usize,
    ) -> Result<Self, BloomError> {
        if max_levels == 0 {
            return Err(BloomError::BadLevelCount(0));
        }
        let bright = bright_pass(pixels, width, height, threshold)?;
        let mut levels = vec![BloomMip {
            width,
            height,
            data: bright,
        }];
        let cap = max_levels.min(MAX_BLOOM_LEVELS);
        while levels.len() < cap {
            let top = &levels[levels.len() - 1];
            if top.width < 2 && top.height < 2 {
                break;
            }
            let (data, w, h) = downsample_2x2(&top.data, top.width, top.height)?;
            // A 1x1 mip downsamples to itself: stop instead of looping.
            if w == top.width && h == top.height {
                break;
            }
            levels.push(BloomMip {
                width: w,
                height: h,
                data,
            });
        }
        Ok(Self { threshold, levels })
    }

    /// Composite back to full resolution: fold the smallest mip upward with
    /// [`upsample_add`]. Returns the full-res bloom to add to the frame.
    pub fn composite(&self) -> Result<Vec<[f32; 3]>, BloomError> {
        let Some((smallest, rest)) = self.levels.split_last() else {
            return Err(BloomError::BadLevelCount(0));
        };
        let mut acc = smallest.data.clone();
        let (mut aw, mut ah) = (smallest.width, smallest.height);
        for level in rest.iter().rev() {
            acc = upsample_add(&level.data, level.width, level.height, &acc, aw, ah)?;
            aw = level.width;
            ah = level.height;
        }
        Ok(acc)
    }
}

// ---------------------------------------------------------------------------
// TAA jitter: Halton-2/3 sequence offsets per frame index.
// ---------------------------------------------------------------------------

/// Radical inverse of `index` in `base` (1-indexed callers pass `frame + 1`
/// so frame 0 leaves the origin). Pure textbook math, deterministic.
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

/// Per-frame TAA jitter in pixel units, centered on `[-0.5, 0.5)`: x from
/// Halton base 2, y from Halton base 3 (the standard 2D low-discrepancy
/// pairing). Index wraps every `u32::MAX` frames; the sequence position
/// depends only on the frame number, never on wall time.
pub struct TaaJitter;

impl TaaJitter {
    /// Jitter offset for `frame` in pixels, centered on `[-0.5, 0.5)`.
    pub fn offset(frame: u64) -> [f32; 2] {
        let i = frame.wrapping_add(1) as u32;
        [halton(i, 2) - 0.5, halton(i, 3) - 0.5]
    }

    /// Jitter for `frame` in DirectX/WebGPU NDC units (`offset * 2 / res`):
    /// the value [`TaaJitter::apply_to_proj`] adds to the projection matrix.
    /// Zero resolution (suspended swap path) yields zero jitter.
    pub fn ndc(frame: u64, width: u32, height: u32) -> [f32; 2] {
        if width == 0 || height == 0 {
            return [0.0, 0.0];
        }
        let [jx, jy] = Self::offset(frame);
        [jx * 2.0 / width as f32, jy * 2.0 / height as f32]
    }

    /// Apply an NDC jitter to a projection matrix for the `v1_gpu` TAA pass:
    /// offsets the third-column x/y (the `m[2][0]`, `m[2][1]` perspective
    /// terms), leaving everything else bit-identical.
    pub fn apply_to_proj(proj: glam::Mat4, ndc: [f32; 2]) -> glam::Mat4 {
        let mut out = proj;
        out.z_axis.x += ndc[0];
        out.z_axis.y += ndc[1];
        out
    }
}

// ---------------------------------------------------------------------------
// Grading: exposure/contrast/saturation applied in the tonemap path.
// ---------------------------------------------------------------------------

/// Linear mid-grey pivot for the contrast control.
pub const GRADE_MID_GREY: f32 = 0.18;

/// Grade failures: typed, no panics on caller data.
#[derive(Clone, PartialEq, Debug)]
pub enum GradeError {
    /// A field is NaN/infinite; which field carried.
    NonFinite(&'static str),
    /// Exposure is not finite and positive; value carried.
    BadExposure(f32),
    /// Contrast is negative or non-finite; value carried.
    BadContrast(f32),
    /// Saturation is negative or non-finite; value carried.
    BadSaturation(f32),
}

impl fmt::Display for GradeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(w) => write!(f, "grade {w} is not finite"),
            Self::BadExposure(v) => write!(f, "grade exposure {v} is not positive finite"),
            Self::BadContrast(v) => write!(f, "grade contrast {v} is not finite non-negative"),
            Self::BadSaturation(v) => {
                write!(f, "grade saturation {v} is not finite non-negative")
            }
        }
    }
}

impl std::error::Error for GradeError {}

/// Color grade applied in linear space before the R1 tonemap: exposure
/// scales, contrast pivots about [`GRADE_MID_GREY`] per channel, saturation
/// lerps between Rec.709 luma and the color. Defaults (1, 1, 1) are the
/// bit-exact identity (each stage short-circuits on 1.0).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GradeSettings {
    /// Pre-tonemap multiplier, `> 0`.
    pub exposure: f32,
    /// Pivot contrast about mid-grey, `>= 0`.
    pub contrast: f32,
    /// Luma/color blend, `>= 0` (1 = unchanged, 0 = greyscale).
    pub saturation: f32,
}

impl Default for GradeSettings {
    fn default() -> Self {
        Self {
            exposure: 1.0,
            contrast: 1.0,
            saturation: 1.0,
        }
    }
}

impl GradeSettings {
    /// Validate ranges before the grade reaches any uniform or pixel.
    pub fn new(exposure: f32, contrast: f32, saturation: f32) -> Result<Self, GradeError> {
        if !exposure.is_finite() || !contrast.is_finite() || !saturation.is_finite() {
            let which = if !exposure.is_finite() {
                "exposure"
            } else if !contrast.is_finite() {
                "contrast"
            } else {
                "saturation"
            };
            return Err(GradeError::NonFinite(which));
        }
        if exposure <= 0.0 {
            return Err(GradeError::BadExposure(exposure));
        }
        if contrast < 0.0 {
            return Err(GradeError::BadContrast(contrast));
        }
        if saturation < 0.0 {
            return Err(GradeError::BadSaturation(saturation));
        }
        Ok(Self {
            exposure,
            contrast,
            saturation,
        })
    }

    /// Grade one linear-RGB texel. Exact identity at defaults: each stage
    /// short-circuits on 1.0 rather than rounding through the pivot.
    pub fn apply_linear(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut c = rgb;
        if self.exposure != 1.0 {
            c = [
                c[0] * self.exposure,
                c[1] * self.exposure,
                c[2] * self.exposure,
            ];
        }
        if self.contrast != 1.0 {
            c = [
                ((c[0] - GRADE_MID_GREY) * self.contrast + GRADE_MID_GREY).max(0.0),
                ((c[1] - GRADE_MID_GREY) * self.contrast + GRADE_MID_GREY).max(0.0),
                ((c[2] - GRADE_MID_GREY) * self.contrast + GRADE_MID_GREY).max(0.0),
            ];
        }
        if self.saturation != 1.0 {
            let l = luminance(c);
            c = [
                (l + self.saturation * (c[0] - l)).max(0.0),
                (l + self.saturation * (c[1] - l)).max(0.0),
                (l + self.saturation * (c[2] - l)).max(0.0),
            ];
        }
        c
    }

    /// Grade-then-tonemap: the tonemap-path application. Grades in linear
    /// space, then runs the R1 Bevy-ported ACES fit plus sRGB encode.
    /// `v1_gpu` evaluates the same order (grade uniforms, then the shared
    /// tonemap chunk). Output: sRGB, finite for finite input.
    pub fn tonemap(&self, linear: [f32; 3]) -> [f32; 3] {
        linear_to_srgb(aces_fitted(self.apply_linear(linear)))
    }

    /// Grade as a GPU uniform `[exposure, contrast, saturation, 0]` for the
    /// `v1_gpu` post pass.
    pub fn to_uniform(&self) -> [f32; 4] {
        [self.exposure, self.contrast, self.saturation, 0.0]
    }
}

// ---------------------------------------------------------------------------
// Tests: splits tile the range, probe blend matches corners, bloom energy,
// Halton values, grade identity, determinism x2, failure cases.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::cube;

    fn approx(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    fn total(pixels: &[[f32; 3]]) -> f64 {
        let mut acc = 0.0f64;
        for p in pixels {
            acc += f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]);
        }
        acc
    }

    #[test]
    fn cascade_splits_cover_range_without_gaps() {
        // Exact assertion: endpoints reproduce the inputs, every interior
        // split is strictly inside its neighbors, so [near, far) tiles with
        // no gap a sample could fall through and no overlap double-covering.
        let (near, far) = (0.5f32, 60.0f32);
        let splits = practical_splits(near, far).expect("valid range splits");
        assert_eq!(splits[0], near, "first split is the camera near");
        assert_eq!(splits[3], far, "last split is the camera far");
        for pair in splits.windows(2) {
            assert!(
                pair[0] < pair[1],
                "splits strictly increase, got {splits:?}"
            );
        }
        // Selection tiles: a depth just below each boundary lands in the
        // earlier cascade, exactly on it in the later one (half-open).
        let set = CascadeSet::new(near, far, [2.0, 6.0, 14.0], [0.002, 0.004, 0.008], 16)
            .expect("valid cascade set");
        for i in 0..3 {
            assert_eq!(set.cascade_index(set.levels[i].near), i.min(2));
            if i < 2 {
                assert_eq!(set.cascade_index(set.levels[i].far), i + 1);
            }
        }
        // Far beyond the range stays in the last cascade, never out of band.
        assert_eq!(set.cascade_index(far * 10.0), 2);
        // Practical blend sits between the pure placements (lambda = 0.5).
        for (i, split) in splits.iter().enumerate().take(3).skip(1) {
            let t = i as f32 / 3.0;
            let log = near * (far / near).powf(t);
            let uni = near + (far - near) * t;
            let lo = log.min(uni);
            let hi = log.max(uni);
            assert!(
                *split >= lo && *split <= hi,
                "split {i} blends log/uni, got {split} in [{lo}, {hi}]"
            );
        }
    }

    #[test]
    fn cascade_bake_and_select_samples() {
        // Overhead light over the unit cube: the near cascade shadows the
        // interior point and lights the point above, through the R2 path
        // with per-level bias.
        let mesh = cube();
        let mut set = CascadeSet::new(0.5, 60.0, [4.0, 8.0, 16.0], [0.002, 0.004, 0.008], 32)
            .expect("valid cascade set");
        // Unbaked levels are lit (no occluder information, never dark).
        assert_eq!(
            set.shadow_factor(0.6, [0.0, 0.4, 0.0])
                .expect("unbaked samples"),
            1.0
        );
        set.bake_all(&mesh, [0.0, 1.0, 0.0], [0.0, 0.0, 0.0], 32)
            .expect("cascades bake");
        for level in &set.levels {
            assert!(
                level
                    .map
                    .as_ref()
                    .is_some_and(|m| m.depths.iter().any(|d| *d < 1.0)),
                "each cascade must write depth texels"
            );
        }
        let lit = set
            .shadow_factor(0.6, [0.0, 0.6, 0.0])
            .expect("above-face samples");
        let dark = set
            .shadow_factor(0.6, [0.0, 0.4, 0.0])
            .expect("below-face samples");
        assert_eq!(lit, 1.0, "point above the caster is lit");
        assert_eq!(dark, 0.0, "point behind the stored depth is shadowed");
        // A deep view depth routes to the far cascade and still samples.
        let deep = set
            .shadow_factor(55.0, [0.0, 0.4, 0.0])
            .expect("deep samples");
        assert_eq!(set.cascade_index(55.0), 2);
        assert!((0.0..=1.0).contains(&deep));
    }

    #[test]
    fn probe_trilinear_matches_manual_corner_blend() {
        // 2x2x2 grid with corner values k/7: the center sample must equal the
        // hand-computed mean, and each corner sample must reproduce its probe
        // exactly (weights collapse to one corner).
        let dims = [2u32, 2, 2];
        let grid = ProbeGrid::bake([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], dims, |p| {
            [p[0] + 2.0 * p[1] + 4.0 * p[2], 0.0, 0.0]
        })
        .expect("2x2x2 grid bakes");
        assert_eq!(grid.samples.len(), 8);
        // Manual corner blend at the center: equal weights, mean of 8.
        let manual = total(&grid.samples) / 8.0;
        let center = sample_trilinear(&grid, [0.5, 0.5, 0.5]).expect("center samples");
        let got = f64::from(center[0]) + f64::from(center[1]) + f64::from(center[2]);
        assert!(
            approx(got as f32, manual as f32, 1e-5),
            "center trilinear {got} must match manual corner mean {manual}"
        );
        assert_eq!(center[1], 0.0);
        assert_eq!(center[2], 0.0);
        // Corners reproduce their probes bit-exactly.
        for iz in 0..2 {
            for iy in 0..2 {
                for ix in 0..2 {
                    let pos = [ix as f32, iy as f32, iz as f32];
                    let s = sample_trilinear(&grid, pos).expect("corner samples");
                    let want = grid.samples[(iz * 2 + iy) * 2 + ix];
                    assert_eq!(s, want, "corner {pos:?} reproduces its probe");
                }
            }
        }
        // Edge clamp: outside the grid returns the edge value, never an error.
        let edge = sample_trilinear(&grid, [10.0, -3.0, 0.5]).expect("edge clamps");
        assert!(edge.iter().all(|c| c.is_finite()));
        // Environment bake: every probe equals the six-axis prefilter mean.
        let env = EnvMap {
            width: 2,
            height: 1,
            pixels: vec![[1.0, 0.5, 0.25], [0.0, 0.0, 0.0]],
        };
        let baked = ProbeGrid::bake_from_env(&env, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [2, 1, 1])
            .expect("env grid bakes");
        assert_eq!(baked.samples.len(), 2);
        assert_eq!(baked.samples[0], baked.samples[1]);
        let s = sample_trilinear(&baked, [0.25, 0.0, 0.0]).expect("env samples");
        assert!(s.iter().all(|c| c.is_finite() && *c >= 0.0));
    }

    #[test]
    fn bloom_factors_conserve_energy_directionally() {
        // Documented assertion: the bright pass only removes energy
        // (sum(out) <= sum(in)), and the 2x2 box downsample conserves it
        // directionally — sum(dst) * 4 == sum(src) within float rounding for
        // even dimensions (every source texel contributes exactly once), and
        // sum(dst) <= sum(src) always. Upsample-add returns exactly the
        // summed inputs (nearest texel + base, no resampling invention).
        let w = 8u32;
        let h = 8u32;
        let mut src = Vec::with_capacity(64);
        for y in 0..8 {
            for x in 0..8 {
                // Bright spot plus a dim floor: threshold must cut the floor.
                let hot = if (3..5).contains(&x) && (3..5).contains(&y) {
                    4.0
                } else {
                    0.05
                };
                src.push([hot, hot * 0.5, hot * 0.25]);
            }
        }
        let before = total(&src);
        let bright = bright_pass(&src, w, h, 0.5).expect("bright pass works");
        let kept = total(&bright);
        assert!(
            kept < before,
            "threshold must remove energy ({kept} < {before})"
        );
        assert!(kept > 0.0, "the hot spot must survive the threshold");
        // Dim-only image thresholds to black: zero energy out.
        let dim = vec![[0.05, 0.05, 0.05]; 64];
        let dark = bright_pass(&dim, w, h, 0.5).expect("dim thresholds");
        assert_eq!(total(&dark), 0.0);
        // Downsample conservation on the bright image (even dims: exact).
        let (down, dw, dh) = downsample_2x2(&bright, w, h).expect("downsamples");
        assert_eq!((dw, dh), (4, 4));
        let down_total = total(&down);
        let rel = ((down_total * 4.0 - kept) / kept).abs();
        assert!(
            rel < 1e-4,
            "box downsample conserves energy: 4*dst {a} vs src {b} (rel {rel})",
            a = down_total * 4.0,
            b = kept
        );
        assert!(
            down_total <= kept + 1e-6,
            "downsample never creates energy ({down_total} <= {kept})"
        );
        // Odd dimensions clamp to the edge instead of failing.
        let (odd, ow, oh) = downsample_2x2(&bright[..7 * 7], 7, 7).expect("odd dims downsample");
        assert_eq!((ow, oh, odd.len()), (3, 3, 9));
        // Chain build + composite round trip keeps full-res shape and stays
        // within the energy budget (composite only adds conserved mips).
        let chain = BloomMipChain::build(&src, w, h, 0.5, 4).expect("chain builds");
        assert!(
            chain.levels.len() >= 2,
            "chain must hold mips, got {}",
            chain.levels.len()
        );
        assert_eq!((chain.levels[0].width, chain.levels[0].height), (w, h));
        let comp = chain.composite().expect("chain composites");
        assert_eq!(comp.len(), src.len());
        assert!(comp
            .iter()
            .all(|p| p.iter().all(|c| c.is_finite() && *c >= 0.0)));
        assert!(
            total(&comp) >= kept,
            "composite must contain at least the bright energy"
        );
    }

    #[test]
    fn taa_jitter_matches_halton_values() {
        // Exact assertion: Halton-2/3 at 1-based indices, centered on
        // [-0.5, 0.5). Frame 0 -> (1/2 - 1/2, 1/3 - 1/2) = (0, -1/6);
        // frame 1 -> (1/4 - 1/2, 2/3 - 1/2) = (-1/4, 1/6);
        // frame 2 -> (3/4 - 1/2, 1/9 - 1/2) = (1/4, -7/18).
        assert!(approx(halton(1, 2), 0.5, 1e-7));
        assert!(approx(halton(2, 2), 0.25, 1e-7));
        assert!(approx(halton(3, 2), 0.75, 1e-7));
        assert!(approx(halton(1, 3), 1.0 / 3.0, 1e-7));
        assert!(approx(halton(2, 3), 2.0 / 3.0, 1e-7));
        assert!(approx(halton(3, 3), 1.0 / 9.0, 1e-7));
        let j0 = TaaJitter::offset(0);
        assert!(approx(j0[0], 0.0, 1e-6) && approx(j0[1], -1.0 / 6.0, 1e-6));
        let j1 = TaaJitter::offset(1);
        assert!(approx(j1[0], -0.25, 1e-6) && approx(j1[1], 1.0 / 6.0, 1e-6));
        let j2 = TaaJitter::offset(2);
        assert!(approx(j2[0], 0.25, 1e-6) && approx(j2[1], -7.0 / 18.0, 1e-6));
        // The 16-frame sequence stays inside the pixel and is non-repeating.
        let mut seen = Vec::new();
        for f in 0..16u64 {
            let j = TaaJitter::offset(f);
            assert!(
                (-0.5..0.5).contains(&j[0]) && (-0.5..0.5).contains(&j[1]),
                "frame {f} jitter {j:?} stays in the pixel"
            );
            assert!(!seen.contains(&j), "frame {f} jitter repeats");
            seen.push(j);
        }
        // NDC scaling is offset * 2 / res; zero res (suspended) is zero.
        let ndc = TaaJitter::ndc(1, 1920, 1080);
        assert!(approx(ndc[0], j1[0] * 2.0 / 1920.0, 1e-9));
        assert!(approx(ndc[1], j1[1] * 2.0 / 1080.0, 1e-9));
        assert_eq!(TaaJitter::ndc(1, 0, 1080), [0.0, 0.0]);
        // Projection application touches only the perspective terms.
        let proj = glam::Mat4::IDENTITY;
        let j = TaaJitter::apply_to_proj(proj, [0.01, -0.02]);
        assert_eq!((j.z_axis.x, j.z_axis.y), (0.01, -0.02));
        assert_eq!(j.x_axis, proj.x_axis);
        assert_eq!(j.y_axis, proj.y_axis);
        assert_eq!(j.w_axis, proj.w_axis);
        assert_eq!(TaaJitter::apply_to_proj(proj, [0.0, 0.0]), proj);
    }

    #[test]
    fn grading_identity_at_defaults() {
        // Exact assertion: defaults are the bit-exact identity (each stage
        // short-circuits on 1.0), so the graded tonemap at defaults equals
        // the R1 tonemap with no rounding drift.
        let grade = GradeSettings::default();
        assert_eq!(
            grade,
            GradeSettings::new(1.0, 1.0, 1.0).expect("defaults valid")
        );
        for rgb in [
            [0.0, 0.0, 0.0],
            [0.1, 0.35, 0.7],
            [1.0, 1.0, 1.0],
            [4.0, 2.0, 0.5],
        ] {
            assert_eq!(
                grade.apply_linear(rgb),
                rgb,
                "default grade is the identity on {rgb:?}"
            );
            assert_eq!(
                grade.tonemap(rgb),
                linear_to_srgb(aces_fitted(rgb)),
                "default graded tonemap equals the R1 tonemap on {rgb:?}"
            );
        }
        // Non-default grades move the output in the documented direction.
        let up = GradeSettings::new(2.0, 1.0, 1.0).expect("2x exposure valid");
        let a = grade.tonemap([0.4, 0.4, 0.4]);
        let b = up.tonemap([0.4, 0.4, 0.4]);
        assert!(b[0] > a[0] && b[1] > a[1] && b[2] > a[2]);
        let grey = GradeSettings::new(1.0, 1.0, 0.0).expect("greyscale valid");
        let g = grey.apply_linear([0.6, 0.2, 0.1]);
        assert!(
            approx(g[0], g[1], 1e-6) && approx(g[1], g[2], 1e-6),
            "zero saturation is greyscale, got {g:?}"
        );
        let punch = GradeSettings::new(1.0, 2.0, 1.0).expect("contrast valid");
        let lo = punch.apply_linear([0.05, 0.05, 0.05]);
        let hi = punch.apply_linear([0.8, 0.8, 0.8]);
        assert!(
            lo[0] < 0.05 && hi[0] > 0.8,
            "contrast spreads about mid-grey"
        );
        // GPU uniform packs the three factors plus a zero pad.
        assert_eq!(up.to_uniform(), [2.0, 1.0, 1.0, 0.0]);
    }

    #[test]
    fn v2_lighting_is_deterministic() {
        // Same complete inputs twice give bit-identical outputs (x2).
        let mesh = cube();
        let build_set = || {
            let mut set = CascadeSet::new(0.5, 60.0, [4.0, 8.0, 16.0], [0.002, 0.004, 0.008], 32)
                .expect("valid cascade set");
            set.bake_all(&mesh, [0.0, 1.0, 0.0], [0.0, 0.0, 0.0], 32)
                .expect("cascades bake");
            set
        };
        let (s1, s2) = (build_set(), build_set());
        assert_eq!(s1, s2, "cascade bake is bit-identical twice");
        assert_eq!(
            s1.shadow_factor(0.6, [0.0, 0.4, 0.0]),
            s2.shadow_factor(0.6, [0.0, 0.4, 0.0])
        );
        let bake_grid = || {
            ProbeGrid::bake([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [2, 2, 2], |p| {
                [p[0] + p[1], p[1] + p[2], p[2] + p[0]]
            })
            .expect("probe grid bakes")
        };
        let (g1, g2) = (bake_grid(), bake_grid());
        assert_eq!(g1, g2, "probe bake is bit-identical twice");
        assert_eq!(
            sample_trilinear(&g1, [0.3, 0.7, 0.5]),
            sample_trilinear(&g2, [0.3, 0.7, 0.5])
        );
        let src: Vec<[f32; 3]> = (0..64).map(|i| [i as f32 * 0.1, 0.2, 0.05]).collect();
        let (c1, c2) = (
            BloomMipChain::build(&src, 8, 8, 0.5, 4).expect("chain builds"),
            BloomMipChain::build(&src, 8, 8, 0.5, 4).expect("chain builds"),
        );
        assert_eq!(c1, c2, "bloom chain is bit-identical twice");
        assert_eq!(
            c1.composite().expect("composites"),
            c2.composite().expect("composites")
        );
        assert_eq!(TaaJitter::offset(7), TaaJitter::offset(7));
        assert_eq!(TaaJitter::ndc(7, 1920, 1080), TaaJitter::ndc(7, 1920, 1080));
        let grade = GradeSettings::new(1.5, 1.2, 0.8).expect("grade valid");
        assert_eq!(
            grade.apply_linear([0.4, 0.3, 0.2]),
            grade.apply_linear([0.4, 0.3, 0.2])
        );
        assert_eq!(
            grade.tonemap([0.4, 0.3, 0.2]),
            grade.tonemap([0.4, 0.3, 0.2])
        );
    }

    #[test]
    fn invalid_inputs_fail_loudly() {
        assert!(matches!(
            practical_splits(60.0, 0.5),
            Err(CascadeError::BadRange { .. })
        ));
        assert!(matches!(
            practical_splits(0.0, 60.0),
            Err(CascadeError::BadRange { .. })
        ));
        assert!(matches!(
            practical_splits(f32::NAN, 60.0),
            Err(CascadeError::NonFinite(_))
        ));
        assert!(matches!(
            CascadeSet::new(0.5, 60.0, [-1.0, 8.0, 16.0], [0.002, 0.004, 0.008], 32),
            Err(CascadeError::BadExtent(_))
        ));
        assert!(matches!(
            CascadeSet::new(0.5, 60.0, [4.0, 8.0, 16.0], [0.002, -1.0, 0.008], 32),
            Err(CascadeError::BadBias(_))
        ));
        assert!(matches!(
            CascadeSet::new(0.5, 60.0, [4.0, 8.0, 16.0], [0.002, 0.004, 0.008], 4),
            Err(CascadeError::BadSize(4))
        ));
        assert!(matches!(
            ProbeGrid::bake([0.0; 3], [1.0; 3], [0, 2, 2], |p| p),
            Err(ProbeError::BadDims(_))
        ));
        assert!(matches!(
            ProbeGrid::bake([0.0; 3], [0.0, 1.0, 1.0], [2, 2, 2], |p| p),
            Err(ProbeError::BadSpacing(_))
        ));
        assert!(matches!(
            ProbeGrid::bake([f32::NAN, 0.0, 0.0], [1.0; 3], [2, 2, 2], |p| p),
            Err(ProbeError::NonFinite(_))
        ));
        assert!(matches!(
            ProbeGrid::bake([0.0; 3], [1.0; 3], [64, 64, 64], |p| p),
            Err(ProbeError::TooManyProbes { .. })
        ));
        let grid = ProbeGrid::bake([0.0; 3], [1.0; 3], [2, 2, 2], |_| [0.2, 0.2, 0.2])
            .expect("probe grid bakes");
        assert!(matches!(
            sample_trilinear(&grid, [f32::INFINITY, 0.0, 0.0]),
            Err(ProbeError::NonFinite(_))
        ));
        let src = vec![[0.3, 0.3, 0.3]; 16];
        assert!(matches!(
            bright_pass(&src, 4, 4, -1.0),
            Err(BloomError::BadThreshold(_))
        ));
        assert!(matches!(
            bright_pass(&src, 4, 5, 0.5),
            Err(BloomError::LengthMismatch { .. })
        ));
        assert!(matches!(
            bright_pass(&src, 0, 4, 0.5),
            Err(BloomError::BadDims { .. })
        ));
        assert!(matches!(
            BloomMipChain::build(&src, 4, 4, 0.5, 0),
            Err(BloomError::BadLevelCount(0))
        ));
        assert!(matches!(
            GradeSettings::new(0.0, 1.0, 1.0),
            Err(GradeError::BadExposure(_))
        ));
        assert!(matches!(
            GradeSettings::new(1.0, -1.0, 1.0),
            Err(GradeError::BadContrast(_))
        ));
        assert!(matches!(
            GradeSettings::new(1.0, 1.0, -1.0),
            Err(GradeError::BadSaturation(_))
        ));
        assert!(matches!(
            GradeSettings::new(f32::NAN, 1.0, 1.0),
            Err(GradeError::NonFinite(_))
        ));
    }
}
