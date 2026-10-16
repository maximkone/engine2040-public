//! fog_vol: analytic volumetric fog plus one cloud slice.
//!
//! Owner: render lane. Caller: [`crate::volumetric_hint`] in the crate root,
//! which reads march steps plus the cloud toggle for the window forward pass;
//! the pass itself binds [`VOL_PASS`] next. Units: meters for depth, heights,
//! falloff and cloud altitude; pixels for frame sizes; milliseconds in
//! receipts. Fog color is the sky horizon in linear HDR, shared with the
//! height-fog lane; output stays linear, the blit tone-maps once.
//! Determinism: row-major integer orders, fixed march tables,
//! single-precision math; the same complete inputs give identical bytes on one
//! toolchain, and the stepped march meets the slab form within
//! [`VOL_PARITY_TOL`] on flat fields. Presentation only: nothing here enters
//! the simulation hash. Failures reuse the height-lane error so the forward
//! bind sees one error type; the headless twin names the window device bind
//! while the window build runs the pass for real with zero validation errors.
//!
//! Quality hook: [`config_for_quality`] derives march steps, slice scale and
//! the cloud toggle from the quality table, so the lowest tier marches four
//! steps with no cloud while the top tier marches sixteen with a full-res
//! slice. The embedded [`VOL_WGSL`] bytes are the device twin of the march.

use std::time::Instant;

// ---------------------------------------------------------------------------
// Pass names, resources and budgets: the forward-pass contract.
// ---------------------------------------------------------------------------

/// Graph pass name for the volumetric compute.
pub const VOL_PASS: &str = "fog-vol";
/// Graph resource name for the volumetric factor buffer the forward pass reads.
pub const VOL_RESOURCE: &str = "fog_vol_factor";
/// Posted GPU budget for the volumetric march at full HD (evidence gate enforces).
pub const VOL_BUDGET_MS: f64 = 0.6;
/// Posted GPU budget for the cloud slice at full HD.
pub const CLOUD_BUDGET_MS: f64 = 0.4;
/// Guard in the slab span denominator (mirrors the forward span guard).
pub const VOL_EPS: f32 = 0.0001;
/// Guard on the cloud-plane ray denominator (mirrors the forward sky guard).
pub const CLOUD_EPS: f32 = 0.00001;
/// Optical-depth clamp shared with the forward shader (keeps `exp` finite).
pub const VOL_TAU_MAX: f32 = 80.0;
/// Stepped-march/slab parity tolerance in absolute factor units.
pub const VOL_PARITY_TOL: f32 = 0.02;
/// Sky-ramp tolerance for the volumetric composite in absolute channel units.
pub const VOL_SKY_RAMP_TOL: f32 = 0.02;
/// Fewest march steps any tier takes (count).
pub const VOL_STEPS_MIN: u8 = 4;
/// Most march steps any tier takes (count).
pub const VOL_STEPS_MAX: u8 = 16;
/// Coarsest cloud-slice divisor any tier takes (ratio).
pub const VOL_SLICE_DIV_MAX: u8 = 4;
/// Lowest accepted cloud altitude in meters.
pub const CLOUD_ALT_MIN_M: f32 = -64.0;
/// Highest accepted cloud altitude in meters.
pub const CLOUD_ALT_MAX_M: f32 = 400.0;
/// Cleanest accepted cloud density in per-meter extinction units.
pub const CLOUD_DENSITY_MIN: f32 = 0.0;
/// Thickest accepted cloud density in per-meter extinction units.
pub const CLOUD_DENSITY_MAX: f32 = 4.0;
/// World size of one noise cell in meters.
pub const CLOUD_CELL_M: f32 = 60.0;
/// Largest transient the aliasing plan must hold for the factor buffer plus
/// the largest tier slice at full HD, plus headroom.
pub const VOL_TRANSIENT_BUDGET_BYTES: u64 = 20_000_000;
/// Bytes per volumetric texel on the device (single float, full res).
pub const VOL_BYTES_PER_TEXEL: u64 = 4;
/// Byte size of the packed volumetric uniform (the shader struct twin; the
/// pack lands with the window bind).
pub const VOL_UNIFORM_BYTES: u64 = 96;
/// Embedded device shader bytes (the march plus the slice, one file, entry
/// variants share the helpers; the window build pipelines them next).
pub const VOL_WGSL: &str = include_str!("shaders/fog_vol.wgsl");

/// Map a look-frame failure into the shared fog input error.
fn look_err(e: crate::gtao::LookError) -> crate::fog::FogError {
    crate::fog::FogError::Invalid(format!("fog-vol look frame: {e}"))
}

// ---------------------------------------------------------------------------
// Volumetric params: height extinction plus sun and one stratus slice.
// ---------------------------------------------------------------------------

/// Volumetric inputs: height extinction shared with the height-fog lane plus
/// a sun for forward scatter and one stratus slice. Validated up front.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct VolParams {
    /// Fog color in linear HDR (pre-exposure), the sky horizon.
    pub fog_color: [f32; 3],
    /// Extinction density per meter, height-lane range.
    pub density_per_m: f32,
    /// Height falloff in meters, height-lane range.
    pub falloff_m: f32,
    /// Fog base height in meters, height-lane range.
    pub base_height_m: f32,
    /// Fog cutoff distance in meters, height-lane range.
    pub max_distance_m: f32,
    /// Sun direction toward the sun, finite nonzero (normalized on use).
    pub sun_dir: [f32; 3],
    /// Sun color in linear HDR, finite nonnegative.
    pub sun_color: [f32; 3],
    /// Cloud slice altitude in meters, `CLOUD_ALT_MIN_M..=CLOUD_ALT_MAX_M`.
    pub cloud_alt_m: f32,
    /// Cloud coverage in `0..=1`; zero clears the slice everywhere.
    pub cloud_coverage: f32,
    /// Cloud density in per-meter extinction, `CLOUD_DENSITY_MIN..=MAX`.
    pub cloud_density: f32,
    /// Exposure anchor in EV100, must be finite (carried for the bind).
    pub exposure_ev100: f32,
}

/// One range check with the field name in the failure.
fn check_range(
    value: f32,
    lo: f32,
    hi: f32,
    name: &'static str,
) -> Result<(), crate::fog::FogError> {
    if !value.is_finite() || value < lo || value > hi {
        return Err(crate::fog::FogError::Invalid(format!(
            "fog-vol {name} {value} outside {lo}..={hi}"
        )));
    }
    Ok(())
}

/// One color check: finite and nonnegative on every channel.
fn check_rgb(value: [f32; 3], name: &'static str) -> Result<(), crate::fog::FogError> {
    if !value.iter().all(|c| c.is_finite()) || value.iter().any(|c| *c < 0.0) {
        return Err(crate::fog::FogError::Invalid(format!(
            "fog-vol {name} {value:?} is not finite nonnegative"
        )));
    }
    Ok(())
}

/// Validate a frame size against the shared look bound before allocation.
fn check_dims(width_px: u32, height_px: u32) -> Result<(), crate::fog::FogError> {
    let bound: u32 = crate::gtao::MAX_LOOK_DIM;
    if width_px == 0 || height_px == 0 || width_px > bound || height_px > bound {
        return Err(crate::fog::FogError::Invalid(format!(
            "fog-vol frame {width_px}x{height_px} outside 1x1..={bound}x{bound}"
        )));
    }
    Ok(())
}

impl VolParams {
    /// Still-air day default: height extinction mirrors the height-fog day
    /// values, sun high, thin broken stratus at one hundred twenty meters.
    pub fn day() -> Self {
        Self {
            fog_color: [0.5, 0.6, 0.7],
            density_per_m: 0.02,
            falloff_m: 8.0,
            base_height_m: 0.0,
            max_distance_m: 500.0,
            sun_dir: [0.3, 0.8, 0.2],
            sun_color: [1.0, 0.95, 0.9],
            cloud_alt_m: 120.0,
            cloud_coverage: 0.4,
            cloud_density: 0.5,
            exposure_ev100: 12.0,
        }
    }

    /// Validate colors, extinction, heights, distance, sun, cloud, exposure.
    pub fn validate(&self) -> Result<(), crate::fog::FogError> {
        check_rgb(self.fog_color, "fog color")?;
        check_range(
            self.density_per_m,
            crate::fog::FOG_DENSITY_MIN,
            crate::fog::FOG_DENSITY_MAX,
            "density",
        )?;
        check_range(
            self.falloff_m,
            crate::fog::FOG_FALLOFF_MIN_M,
            crate::fog::FOG_FALLOFF_MAX_M,
            "falloff",
        )?;
        check_range(
            self.base_height_m,
            crate::fog::FOG_BASE_MIN_M,
            crate::fog::FOG_BASE_MAX_M,
            "base height",
        )?;
        check_range(
            self.max_distance_m,
            crate::fog::FOG_DISTANCE_MIN_M,
            crate::fog::FOG_DISTANCE_MAX_M,
            "distance",
        )?;
        if crate::fog::normalize3(self.sun_dir).is_none() {
            return Err(crate::fog::FogError::Invalid(format!(
                "fog-vol sun_dir {:?} is not a finite nonzero direction",
                self.sun_dir
            )));
        }
        check_rgb(self.sun_color, "sun color")?;
        check_range(
            self.cloud_alt_m,
            CLOUD_ALT_MIN_M,
            CLOUD_ALT_MAX_M,
            "cloud altitude",
        )?;
        check_range(self.cloud_coverage, 0.0, 1.0, "cloud coverage")?;
        check_range(
            self.cloud_density,
            CLOUD_DENSITY_MIN,
            CLOUD_DENSITY_MAX,
            "cloud density",
        )?;
        if !self.exposure_ev100.is_finite() {
            return Err(crate::fog::FogError::Invalid(
                "fog-vol exposure_ev100 is not finite".into(),
            ));
        }
        Ok(())
    }

    /// Full-res factor transient bytes for a frame (`texels * 4`).
    pub fn factor_bytes(width_px: u32, height_px: u32) -> Result<u64, crate::fog::FogError> {
        check_dims(width_px, height_px)?;
        Ok(u64::from(width_px) * u64::from(height_px) * VOL_BYTES_PER_TEXEL)
    }
}

// ---------------------------------------------------------------------------
// Quality hook: march budget derived from the quality table.
// ---------------------------------------------------------------------------

/// Per-tier march budget: step count, slice divisor and the cloud toggle.
/// Steps rise with the particle budget, the slice refines with output scale
/// and multisampling, clouds ride the ambient-occlusion level.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VolConfig {
    /// March steps per ray (count), `VOL_STEPS_MIN..=VOL_STEPS_MAX`.
    pub steps: u8,
    /// Cloud-slice divisor (ratio); one means full res.
    pub slice_divisor: u8,
    /// Whether the slice marches at all (bool).
    pub cloud_on: bool,
}

impl VolConfig {
    /// Validate steps and divisor before any allocation.
    pub fn validate(&self) -> Result<(), crate::fog::FogError> {
        if self.steps < VOL_STEPS_MIN || self.steps > VOL_STEPS_MAX {
            return Err(crate::fog::FogError::Invalid(format!(
                "fog-vol steps {} outside {VOL_STEPS_MIN}..={VOL_STEPS_MAX}",
                self.steps
            )));
        }
        if self.slice_divisor == 0 || self.slice_divisor > VOL_SLICE_DIV_MAX {
            return Err(crate::fog::FogError::Invalid(format!(
                "fog-vol slice divisor {} outside 1..={VOL_SLICE_DIV_MAX}",
                self.slice_divisor
            )));
        }
        Ok(())
    }
}

/// Derive the march budget from the quality table: the lowest tier marches
/// four steps with no cloud, the middle eight with a half-res slice, the top
/// sixteen with a full-res slice. Reads the table, never copies it.
#[must_use]
pub fn config_for_quality(quality: crate::quality::Quality) -> VolConfig {
    let knobs = crate::quality::knobs(quality);
    let steps: u8 = if knobs.particle_cap <= 512 {
        VOL_STEPS_MIN
    } else if knobs.particle_cap <= 4096 {
        8
    } else {
        VOL_STEPS_MAX
    };
    let slice_divisor: u8 = if knobs.render_scale_pct < 100 {
        VOL_SLICE_DIV_MAX
    } else if knobs.msaa == 0 {
        2
    } else {
        1
    };
    let cloud_on: bool = !matches!(knobs.gtao, crate::quality::Gtao::Off);
    VolConfig {
        steps,
        slice_divisor,
        cloud_on,
    }
}

// ---------------------------------------------------------------------------
// Slab factors: the cheap closed form the march refines.
// ---------------------------------------------------------------------------

/// Reject a non-finite point with its lane name in the failure.
fn check_point(value: [f32; 3], name: &'static str) -> Result<(), crate::fog::FogError> {
    if !value.iter().all(|c| c.is_finite()) {
        return Err(crate::fog::FogError::Invalid(format!(
            "fog-vol {name} {value:?} is not finite"
        )));
    }
    Ok(())
}

/// Closed-form height extinction between two points: one slab at the fragment
/// height, cutoff-clamped, factor `1-exp(-od)`. Returns `0.0` at zero distance
/// and nears `1.0` down long rays through thick air.
pub fn vol_factor_finite(
    params: &VolParams,
    eye: [f32; 3],
    point: [f32; 3],
) -> Result<f32, crate::fog::FogError> {
    params.validate()?;
    check_point(eye, "eye")?;
    check_point(point, "point")?;
    let dx: f32 = point[0] - eye[0];
    let dy: f32 = point[1] - eye[1];
    let dz: f32 = point[2] - eye[2];
    let dist: f32 = (dx * dx + dy * dy + dz * dz)
        .sqrt()
        .min(params.max_distance_m);
    let e: f32 = (-((point[1] - params.base_height_m) / params.falloff_m)).clamp(-8.0, 8.0);
    let tau: f32 = (params.density_per_m * dist * e.exp()).min(VOL_TAU_MAX);
    Ok(1.0 - (-tau).exp())
}

/// Sky-ray twin: the slab out to the cutoff along the ray, so sky pixels fog
/// like the fragment sitting at the cutoff. Level rays inside the fog read
/// opaque; rays leaving clean air read clear. Bad directions fail loudly.
pub fn vol_factor_infinite(
    params: &VolParams,
    eye: [f32; 3],
    dir: [f32; 3],
) -> Result<f32, crate::fog::FogError> {
    params.validate()?;
    check_point(eye, "eye")?;
    let ray: [f32; 3] = crate::fog::normalize3(dir).ok_or_else(|| {
        crate::fog::FogError::Invalid(format!(
            "fog-vol dir {dir:?} is not a finite nonzero direction"
        ))
    })?;
    let point: [f32; 3] = [
        eye[0] + ray[0] * params.max_distance_m,
        eye[1] + ray[1] * params.max_distance_m,
        eye[2] + ray[2] * params.max_distance_m,
    ];
    vol_factor_finite(params, eye, point)
}

/// Combine two extinction factors (height lane plus volumetric) as independent
/// transmittances: `1 - (1-a) * (1-b)`. Either lane clear leaves the other
/// untouched; either opaque wins. Inputs clamp to `0..=1`.
#[must_use]
pub fn combine_factors(a: f32, b: f32) -> f32 {
    let kept_a: f32 = 1.0 - a.clamp(0.0, 1.0);
    let kept_b: f32 = 1.0 - b.clamp(0.0, 1.0);
    1.0 - kept_a * kept_b
}

// ---------------------------------------------------------------------------
// Cloud slice: one billow plane over static deterministic noise.
// ---------------------------------------------------------------------------

/// Deterministic cell hash in `0..1` from integer lattice coords: an integer
/// mix plus a mantissa build, no float cast anywhere on the path.
fn hash01(x: u32, z: u32) -> f32 {
    let mut h: u32 = x
        .wrapping_mul(0x85EB_CA6B)
        .wrapping_add(z.wrapping_mul(0xC2B2_AE35));
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 16;
    f32::from_bits(0x3F80_0000u32 | (h >> 9)) - 1.0
}

/// Value noise in `0..=1` over slice coords: hashed lattice corners with a
/// smoothstep blend. Static (no clock); drift arrives with the wind uniform.
fn vnoise(x: f32, z: f32) -> f32 {
    let ix: f32 = x.floor();
    let iz: f32 = z.floor();
    let fx: f32 = x - ix;
    let fz: f32 = z - iz;
    let sx: f32 = fx * fx * (3.0 - 2.0 * fx);
    let sz: f32 = fz * fz * (3.0 - 2.0 * fz);
    let a: f32 = hash01(ix.to_bits(), iz.to_bits());
    let b: f32 = hash01((ix + 1.0).to_bits(), iz.to_bits());
    let c: f32 = hash01(ix.to_bits(), (iz + 1.0).to_bits());
    let d: f32 = hash01((ix + 1.0).to_bits(), (iz + 1.0).to_bits());
    let ab: f32 = a + (b - a) * sx;
    let cd: f32 = c + (d - c) * sx;
    ab + (cd - ab) * sz
}

/// Three-octave fractal noise in `0..=1` for the slice: base billow plus two
/// detail octaves with fixed offsets so octaves never align.
fn fbm(x: f32, z: f32) -> f32 {
    let base: f32 = vnoise(x, z);
    let mid: f32 = vnoise(x * 2.03 + 13.7, z * 2.03 + 7.1);
    let fine: f32 = vnoise(x * 4.07 + 47.3, z * 4.07 + 29.7);
    (base * 0.55 + mid * 0.28 + fine * 0.17).clamp(0.0, 1.0)
}

/// Shape raw noise by coverage: below `1 - coverage` the sky stays clear,
/// above it density ramps to full over the coverage band.
fn cover_shaped(noise: f32, coverage: f32) -> f32 {
    ((noise - (1.0 - coverage)) / coverage.max(VOL_EPS)).clamp(0.0, 1.0)
}

/// Cloud optical depth for a normalized ray: plane crossing plus shaped
/// billow. Assumes validated params and a unit ray; returns `0.0` off slice.
fn cloud_od(params: &VolParams, eye: [f32; 3], dir: [f32; 3], dist: f32) -> f32 {
    if params.cloud_coverage <= 0.0 || params.cloud_density <= 0.0 {
        return 0.0;
    }
    if dir[1].abs() <= CLOUD_EPS {
        return 0.0;
    }
    let cross: f32 = (params.cloud_alt_m - eye[1]) / dir[1];
    if cross < 0.0 || cross > dist {
        return 0.0;
    }
    let shaped: f32 = cover_shaped(
        fbm(
            (eye[0] + dir[0] * cross) / CLOUD_CELL_M,
            (eye[2] + dir[2] * cross) / CLOUD_CELL_M,
        ),
        params.cloud_coverage,
    );
    params.cloud_density * shaped
}

/// One-slice transmittance for a ray: cross the altitude plane, sample the
/// billow at the crossing, shape by coverage. Returns `0.0` when the ray runs
/// level, crosses behind the fragment, or coverage is zero.
pub fn cloud_slice_factor(
    params: &VolParams,
    eye: [f32; 3],
    dir: [f32; 3],
    dist_m: f32,
) -> Result<f32, crate::fog::FogError> {
    params.validate()?;
    check_point(eye, "eye")?;
    let ray: [f32; 3] = crate::fog::normalize3(dir).ok_or_else(|| {
        crate::fog::FogError::Invalid(format!(
            "fog-vol dir {dir:?} is not a finite nonzero direction"
        ))
    })?;
    if !dist_m.is_finite() || dist_m <= 0.0 {
        return Err(crate::fog::FogError::Invalid(format!(
            "fog-vol dist {dist_m} is not positive finite"
        )));
    }
    let dist: f32 = dist_m.min(params.max_distance_m);
    let tau: f32 = cloud_od(params, eye, ray, dist).min(VOL_TAU_MAX);
    Ok(1.0 - (-tau).exp())
}

// ---------------------------------------------------------------------------
// Bake: stepped march plus slice, forward-scatter boosted.
// ---------------------------------------------------------------------------

/// Per-pixel volumetric factor: `0.0` clear, `1.0` fully fogged. Row-major.
#[derive(Clone, PartialEq, Debug)]
pub struct VolField {
    /// Frame width in pixels.
    pub width_px: u32,
    /// Frame height in pixels.
    pub height_px: u32,
    /// Row-major factors in `0..=1`.
    pub factor: Vec<f32>,
    /// Backing fog color (set by the bake; equals the bound sky horizon).
    color: [f32; 3],
}

impl VolField {
    /// Texel count.
    pub fn texels(&self) -> usize {
        self.factor.len()
    }

    /// Deterministic little-endian bytes: row-major factors. Same inputs give
    /// identical bytes on one toolchain.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.factor.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// Fog color this field was baked against (equals the bound sky color).
    pub fn fog_color_hint(&self) -> [f32; 3] {
        self.color
    }
}

/// Mix base radiance toward fog by factor in single precision (`base * (1-f)
/// + fog * f`). Length mismatches fail loudly; output stays linear for the
/// one blit tone-map.
pub fn composite_vol(
    base: &[[f32; 3]],
    factor: &[f32],
    fog: [f32; 3],
) -> Result<Vec<[f32; 3]>, crate::fog::FogError> {
    if base.len() != factor.len() {
        return Err(crate::fog::FogError::Invalid(format!(
            "fog-vol composite base {} mismatches factor {}",
            base.len(),
            factor.len()
        )));
    }
    check_rgb(fog, "composite fog")?;
    Ok(base
        .iter()
        .zip(factor.iter())
        .map(|(b, f)| {
            let t: f32 = f.clamp(0.0, 1.0);
            let kept: f32 = 1.0 - t;
            [
                b[0] * kept + fog[0] * t,
                b[1] * kept + fog[1] * t,
                b[2] * kept + fog[2] * t,
            ]
        })
        .collect())
}

/// Bake volumetric factors from the eye, per-pixel view rays (`dirs`, finite
/// nonzero), view distance (`depth_m`, positive finite) and fragment world
/// height (`height_m`, finite): fixed-step midpoint march of the height
/// extinction plus one cloud-slice crossing, forward-scatter boosted, factor
/// `1-exp(-tau)`. Row-major integer order. Returns the field plus wall-clock
/// bake ms (evidence only, never hashed).
pub fn bake_vol_factors(
    eye: [f32; 3],
    dirs: &[[f32; 3]],
    depth_m: &[f32],
    height_m: &[f32],
    params: &VolParams,
    config: &VolConfig,
    size_px: [u32; 2],
) -> Result<(VolField, f64), crate::fog::FogError> {
    params.validate()?;
    config.validate()?;
    check_dims(size_px[0], size_px[1])?;
    check_point(eye, "eye")?;
    let w: usize = usize::try_from(size_px[0]).map_err(|_| {
        crate::fog::FogError::Invalid(format!(
            "fog-vol width {} exceeds address range",
            size_px[0]
        ))
    })?;
    let h: usize = usize::try_from(size_px[1]).map_err(|_| {
        crate::fog::FogError::Invalid(format!(
            "fog-vol height {} exceeds address range",
            size_px[1]
        ))
    })?;
    let n: usize = w.checked_mul(h).ok_or_else(|| {
        crate::fog::FogError::Invalid(format!("fog-vol frame {w}x{h} overflows address range"))
    })?;
    for (name, len) in [
        ("rays", dirs.len()),
        ("depth", depth_m.len()),
        ("height", height_m.len()),
    ] {
        if len != n {
            return Err(crate::fog::FogError::Invalid(format!(
                "fog-vol {name} length {len} mismatches frame {n}"
            )));
        }
    }
    for (i, d) in depth_m.iter().enumerate() {
        if !d.is_finite() || *d <= 0.0 {
            return Err(crate::fog::FogError::Invalid(format!(
                "fog-vol depth[{i}] value {d} is not positive finite"
            )));
        }
    }
    for (i, v) in height_m.iter().enumerate() {
        if !v.is_finite() {
            return Err(crate::fog::FogError::Invalid(format!(
                "fog-vol height[{i}] value {v} is not finite"
            )));
        }
    }
    let started: Instant = Instant::now();
    let steps_f: f32 = f32::from(config.steps);
    let cutoff: f32 = params.max_distance_m;
    let mut factor: Vec<f32> = Vec::with_capacity(n);
    for row in 0..h {
        for col in 0..w {
            let i: usize = row * w + col;
            let dir: [f32; 3] = crate::fog::normalize3(dirs[i]).ok_or_else(|| {
                crate::fog::FogError::Invalid(format!(
                    "fog-vol rays[{i}] {:?} is not a finite nonzero direction",
                    dirs[i]
                ))
            })?;
            let dist: f32 = depth_m[i].min(cutoff);
            let step_len: f32 = dist / steps_f;
            let mut od: f32 = 0.0;
            for s in 0..config.steps {
                let t: f32 = (f32::from(s) + 0.5) * step_len;
                let hy: f32 = eye[1] + dir[1] * t;
                let e: f32 = (-((hy - params.base_height_m) / params.falloff_m)).clamp(-8.0, 8.0);
                od += params.density_per_m * step_len * e.exp();
            }
            let cloud: f32 = if config.cloud_on {
                cloud_od(params, eye, dir, dist)
            } else {
                0.0
            };
            let glow: f32 = crate::fog::sun_scatter_glow(dir, params.sun_dir)
                .map_err(|e| crate::fog::FogError::Invalid(format!("fog-vol sun lane: {e}")))?;
            let tau: f32 =
                ((od + cloud) * (1.0 + crate::fog::SUN_TINT_STRENGTH * glow)).min(VOL_TAU_MAX);
            factor.push(1.0 - (-tau).exp());
        }
    }
    let ms: f64 = started.elapsed().as_secs_f64() * 1000.0;
    Ok((
        VolField {
            width_px: size_px[0],
            height_px: size_px[1],
            factor,
            color: params.fog_color,
        },
        ms,
    ))
}

// ---------------------------------------------------------------------------
// Graph wiring: volumetric before forward, same discipline as height fog.
// ---------------------------------------------------------------------------

/// Volumetric graph wiring for one frame: pass names, resources and transient
/// bytes. The factor buffer plus the tier slice are production transients the
/// aliasing plan must hold inside [`VOL_TRANSIENT_BUDGET_BYTES`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VolPassPlan {
    /// Graph pass running the volumetric compute.
    pub vol_pass: &'static str,
    /// Factor resource the forward pass reads.
    pub vol_resource: &'static str,
    /// Factor transient bytes (full-res `texels * 4`).
    pub vol_bytes: u64,
    /// Cloud-slice transient bytes at the tier divisor.
    pub slice_bytes: u64,
}

/// Validate the inputs, then describe the pass plus the bytes the aliasing
/// plan must hold. Pure data, no allocation.
pub fn vol_pass_plan(
    params: &VolParams,
    config: &VolConfig,
    size_px: [u32; 2],
) -> Result<VolPassPlan, crate::fog::FogError> {
    params.validate()?;
    config.validate()?;
    let vol_bytes: u64 = VolParams::factor_bytes(size_px[0], size_px[1])?;
    let div: u64 = u64::from(config.slice_divisor);
    let sw: u64 = u64::from(size_px[0]).div_ceil(div).max(1);
    let sh: u64 = u64::from(size_px[1]).div_ceil(div).max(1);
    let slice_bytes: u64 = sw.saturating_mul(sh).saturating_mul(VOL_BYTES_PER_TEXEL);
    let total: u64 = vol_bytes.saturating_add(slice_bytes);
    if total > VOL_TRANSIENT_BUDGET_BYTES {
        return Err(crate::fog::FogError::Invalid(format!(
            "fog-vol transients {total} over budget {VOL_TRANSIENT_BUDGET_BYTES}"
        )));
    }
    Ok(VolPassPlan {
        vol_pass: VOL_PASS,
        vol_resource: VOL_RESOURCE,
        vol_bytes,
        slice_bytes,
    })
}

/// Declare the volumetric pass plus the forward pass on a fresh frame graph:
/// volumetric writes the factor, forward reads it over its color target.
/// Resource flow plus an explicit edge pins volumetric before forward.
pub fn build_vol_graph(
    params: &VolParams,
    config: &VolConfig,
    size_px: [u32; 2],
) -> Result<crate::graph::FrameGraph, crate::fog::FogError> {
    let plan: VolPassPlan = vol_pass_plan(params, config, size_px)?;
    let mut graph: crate::graph::FrameGraph = crate::graph::FrameGraph::new();
    let vol = graph.add_pass(
        crate::graph::PassDesc::new(plan.vol_pass)
            .input("depth")
            .input("vol_rays")
            .input("sky_capture")
            .output(plan.vol_resource)
            .transient(plan.vol_resource, plan.vol_bytes)
            .work(1, 1),
    );
    let forward = graph.add_pass(
        crate::graph::PassDesc::new("forward")
            .input(plan.vol_resource)
            .input("hdr")
            .output("hdr")
            .work(64, 480_000),
    );
    graph
        .add_edge(vol, forward)
        .map_err(|e| crate::fog::FogError::Invalid(format!("fog-vol graph edge: {e}")))?;
    Ok(graph)
}

/// Validate the match frame inputs, declare volumetric plus forward, compile,
/// and check the compiled order runs volumetric before forward. Returns the
/// compiled graph the window bind records on the device.
pub fn compile_vol_for_forward(
    params: &VolParams,
    config: &VolConfig,
    size_px: [u32; 2],
) -> Result<crate::graph::CompiledGraph, crate::fog::FogError> {
    let graph: crate::graph::FrameGraph = build_vol_graph(params, config, size_px)?;
    let compiled: crate::graph::CompiledGraph = graph
        .compile()
        .map_err(|e| crate::fog::FogError::Invalid(format!("fog-vol graph compile: {e}")))?;
    let names: Vec<&str> = compiled
        .order()
        .iter()
        .filter_map(|id| compiled.pass(*id).map(|p| p.name.as_str()))
        .collect();
    if names != [VOL_PASS, "forward"] {
        return Err(crate::fog::FogError::Invalid(format!(
            "fog-vol compiled order {names:?} differs from [\"{VOL_PASS}\", \"forward\"]"
        )));
    }
    Ok(compiled)
}

// ---------------------------------------------------------------------------
// Receipt plus the headless bind seam.
// ---------------------------------------------------------------------------

/// One volumetric frame receipt: buffer sizes plus wall-clock ms for the
/// frame report (ms is evidence only, never hashed).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct VolReceipt {
    /// Frame counter the bind carried.
    pub frame_index: u64,
    /// Volumetric texels resident.
    pub vol_texels: u64,
    /// Wall-clock bake time in ms.
    pub ms: f64,
}

impl VolReceipt {
    /// Posted march-pass budget this receipt is charged against.
    pub fn vol_budget_ms(&self) -> f64 {
        VOL_BUDGET_MS
    }

    /// Posted slice-pass budget this receipt is charged against.
    pub fn cloud_budget_ms(&self) -> f64 {
        CLOUD_BUDGET_MS
    }

    /// Posted combined budget (march plus slice).
    pub fn budget_ms(&self) -> f64 {
        VOL_BUDGET_MS + CLOUD_BUDGET_MS
    }

    /// True while the bake stayed inside the combined budget.
    pub fn within_budget(&self) -> bool {
        self.ms <= self.budget_ms()
    }
}

/// Headless bind seam: validates the frame plus the backing buffer, then names
/// the missing window device bind (never silent success). The CPU bake above
/// is the reference here; the window build records the live pass instead.
pub fn vol_headless_seam(
    field: &VolField,
    frame: &crate::gtao::LookFrame,
) -> Result<VolReceipt, crate::fog::FogError> {
    frame.validate().map_err(look_err)?;
    if frame.viewport_px != [field.width_px, field.height_px] {
        return Err(crate::fog::FogError::Invalid(format!(
            "fog-vol field {}x{} mismatches frame {}x{}",
            field.width_px, field.height_px, frame.viewport_px[0], frame.viewport_px[1]
        )));
    }
    for (i, v) in field.factor.iter().enumerate() {
        if !v.is_finite() || *v < 0.0 || *v > 1.0 {
            return Err(crate::fog::FogError::Invalid(format!(
                "fog-vol factor[{i}] value {v} outside 0..=1"
            )));
        }
    }
    Err(crate::fog::FogError::NoDevice(
        "fog-vol GPU bind needs the window build with a live device plus the forward bind group; the CPU bake is the reference".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::Quality;

    fn flat_frame(
        n: usize,
        depth: f32,
        height: f32,
        dir: [f32; 3],
    ) -> (Vec<[f32; 3]>, Vec<f32>, Vec<f32>) {
        (vec![dir; n], vec![depth; n], vec![height; n])
    }

    fn look_frame_4() -> crate::gtao::LookFrame {
        crate::gtao::LookFrame {
            frame_index: 3,
            viewport_px: [4, 4],
            exposure_ev100: 12.0,
            sun_dir: [0.3, 0.8, 0.2],
        }
    }

    #[test]
    fn vol_day_params_validate() {
        VolParams::day().validate().expect("day defaults validate");
    }

    #[test]
    fn vol_params_reject_bad_inputs() {
        let base = VolParams::day();
        let bad_density = VolParams {
            density_per_m: -0.5,
            ..base
        };
        assert!(bad_density.validate().is_err(), "negative density fails");
        let bad_falloff = VolParams {
            falloff_m: 0.0,
            ..base
        };
        assert!(bad_falloff.validate().is_err(), "zero falloff fails");
        let bad_sun = VolParams {
            sun_dir: [0.0, 0.0, 0.0],
            ..base
        };
        assert!(bad_sun.validate().is_err(), "zero sun dir fails");
        let bad_cover = VolParams {
            cloud_coverage: 1.5,
            ..base
        };
        assert!(bad_cover.validate().is_err(), "coverage over one fails");
        let bad_color = VolParams {
            fog_color: [f32::NAN, 0.0, 0.0],
            ..base
        };
        assert!(bad_color.validate().is_err(), "NaN color fails");
        let bad_alt = VolParams {
            cloud_alt_m: 900.0,
            ..base
        };
        assert!(bad_alt.validate().is_err(), "cloud above range fails");
        let bad_exposure = VolParams {
            exposure_ev100: f32::INFINITY,
            ..base
        };
        assert!(bad_exposure.validate().is_err(), "infinite exposure fails");
        assert!(
            VolParams::factor_bytes(0, 4).is_err(),
            "zero-width dims fail"
        );
        assert_eq!(
            VolParams::factor_bytes(4, 4).expect("small frame sizes"),
            64,
            "4x4 factor bytes are 64"
        );
    }

    #[test]
    fn vol_quality_hook_tiers() {
        let potato = config_for_quality(Quality::Potato);
        let comfort = config_for_quality(Quality::Comfort);
        let high = config_for_quality(Quality::High);
        assert_eq!(
            (potato.steps, potato.slice_divisor, potato.cloud_on),
            (VOL_STEPS_MIN, VOL_SLICE_DIV_MAX, false),
            "lowest tier marches least with no cloud"
        );
        assert_eq!(
            (comfort.steps, comfort.slice_divisor, comfort.cloud_on),
            (8, 2, true),
            "middle tier marches eight with a half slice"
        );
        assert_eq!(
            (high.steps, high.slice_divisor, high.cloud_on),
            (VOL_STEPS_MAX, 1, true),
            "top tier marches most with a full slice"
        );
        assert!(
            potato.steps <= comfort.steps && comfort.steps <= high.steps,
            "steps rise with tier"
        );
        assert!(
            potato.slice_divisor >= comfort.slice_divisor
                && comfort.slice_divisor >= high.slice_divisor,
            "slice refines with tier"
        );
        assert!(
            !potato.cloud_on && comfort.cloud_on && high.cloud_on,
            "clouds switch on above the lowest tier"
        );
        for config in [potato, comfort, high] {
            config.validate().expect("hook configs validate");
        }
        let bad = VolConfig {
            steps: 0,
            slice_divisor: 1,
            cloud_on: false,
        };
        assert!(bad.validate().is_err(), "zero steps fail");
    }

    #[test]
    fn vol_finite_endpoints_and_monotone() {
        let params = VolParams::day();
        let eye: [f32; 3] = [0.0, 2.0, 0.0];
        let here: f32 = vol_factor_finite(&params, eye, eye).expect("zero distance bakes");
        assert_eq!(here, 0.0, "zero distance reads clear");
        let near: f32 = vol_factor_finite(&params, eye, [10.0, 2.0, 0.0]).expect("near bakes");
        let far: f32 = vol_factor_finite(&params, eye, [400.0, 2.0, 0.0]).expect("far bakes");
        assert!(
            here <= near && near <= far && far <= 1.0,
            "factor rises with distance: {here} {near} {far}"
        );
        assert!(far > 0.5, "long day ray fogs hard, got {far}");
        assert!(
            vol_factor_finite(&params, [f32::NAN, 0.0, 0.0], eye).is_err(),
            "NaN eye fails"
        );
    }

    #[test]
    fn vol_infinite_sky_forms() {
        let mut low = VolParams::day();
        low.base_height_m = 10.0;
        let trapped: f32 = vol_factor_infinite(&low, [0.0, 2.0, 0.0], [1.0, 0.0, 0.0])
            .expect("level ray inside fog bakes");
        assert!(
            trapped > 0.99,
            "level ray inside fog never leaves, got {trapped}"
        );
        let day = VolParams::day();
        let up: f32 =
            vol_factor_infinite(&day, [0.0, 50.0, 0.0], [0.0, 1.0, 0.0]).expect("up ray bakes");
        assert!(up < 0.05, "up ray from clean air reads clear, got {up}");
        let down: f32 =
            vol_factor_infinite(&day, [0.0, 50.0, 0.0], [0.0, -1.0, 0.0]).expect("down ray bakes");
        assert!(
            down > 0.99,
            "down ray over bottomless fog reads opaque, got {down}"
        );
        assert!(
            vol_factor_infinite(&day, [0.0, 2.0, 0.0], [0.0, 0.0, 0.0]).is_err(),
            "zero dir fails"
        );
    }

    #[test]
    fn vol_noise_stays_unit_and_anchored() {
        for (x, z) in [(0u32, 0u32), (7u32, 13u32), (999u32, 1u32)] {
            let h: f32 = hash01(x, z);
            assert!(
                (0.0..1.0).contains(&h),
                "hash {x},{z} stays in 0..1, got {h}"
            );
        }
        let corner: f32 = vnoise(3.0, 5.0);
        let hashed: f32 = hash01(3.0f32.to_bits(), 5.0f32.to_bits());
        assert_eq!(corner, hashed, "lattice noise meets its corner hash");
        for u in [0.0, 31.7, -12.25, 400.0] {
            for v in [0.0, 9.5, -3.75, 210.0] {
                let n: f32 = fbm(u, v);
                assert!(
                    (0.0..=1.0).contains(&n),
                    "fbm {u},{v} stays in 0..=1, got {n}"
                );
            }
        }
    }

    #[test]
    fn vol_cloud_slice_rules() {
        let eye: [f32; 3] = [0.0, 2.0, 0.0];
        let up: [f32; 3] = [0.0, 1.0, 0.0];
        let mut params = VolParams::day();
        params.cloud_alt_m = 100.0;
        params.cloud_coverage = 0.0;
        assert_eq!(
            cloud_slice_factor(&params, eye, up, 500.0).expect("clear sky bakes"),
            0.0,
            "zero coverage clears the slice"
        );
        params.cloud_coverage = 0.8;
        assert_eq!(
            cloud_slice_factor(&params, eye, [1.0, 0.0, 0.0], 500.0).expect("level bakes"),
            0.0,
            "level ray never crosses the slice"
        );
        assert_eq!(
            cloud_slice_factor(&params, eye, up, 50.0).expect("short bakes"),
            0.0,
            "crossing behind the fragment misses"
        );
        params.cloud_coverage = 1.0;
        params.cloud_density = CLOUD_DENSITY_MAX;
        let mut best: f32 = 0.0;
        for k in 0u8..32 {
            let shifted: [f32; 3] = [f32::from(k) * 37.0, 2.0, 0.0];
            let hit: f32 = cloud_slice_factor(&params, shifted, up, 500.0).expect("cover bakes");
            best = best.max(hit);
        }
        assert!(best > 0.3, "overcast sky shows cloud somewhere, got {best}");
        assert!(
            cloud_slice_factor(&params, eye, up, -4.0).is_err(),
            "negative distance fails"
        );
    }

    #[test]
    fn vol_bake_meets_slab_on_flat_field() {
        let mut params = VolParams::day();
        params.sun_dir = [0.0, 1.0, 0.0];
        let config = config_for_quality(Quality::High);
        let (dirs, depth, height) = flat_frame(16, 20.0, 5.0, [1.0, 0.0, 0.0]);
        let eye: [f32; 3] = [0.0, 5.0, 0.0];
        let (field, _) = bake_vol_factors(eye, &dirs, &depth, &height, &params, &config, [4, 4])
            .expect("flat field bakes");
        assert_eq!(field.texels(), 16, "4x4 bakes sixteen factors");
        let slab: f32 = vol_factor_finite(&params, eye, [20.0, 5.0, 0.0]).expect("slab bakes");
        for (i, f) in field.factor.iter().enumerate() {
            assert!(
                (f - slab).abs() < 0.005,
                "march meets slab at texel {i}: {f} vs {slab}"
            );
        }
        let (again, _) = bake_vol_factors(eye, &dirs, &depth, &height, &params, &config, [4, 4])
            .expect("flat field bakes twice");
        assert_eq!(
            field.to_bytes(),
            again.to_bytes(),
            "same inputs bake identical bytes"
        );
    }

    #[test]
    fn vol_bake_rejects_bad_buffers() {
        let params = VolParams::day();
        let config = config_for_quality(Quality::Comfort);
        let eye: [f32; 3] = [0.0, 5.0, 0.0];
        let (dirs, depth, height) = flat_frame(16, 20.0, 5.0, [1.0, 0.0, 0.0]);
        assert!(
            bake_vol_factors(eye, &dirs, &depth, &height, &params, &config, [4, 3]).is_err(),
            "shape/size mismatch fails"
        );
        assert!(
            bake_vol_factors(eye, &dirs, &depth, &height, &params, &config, [0, 0]).is_err(),
            "zero frame fails"
        );
        let mut bad_depth = depth.clone();
        bad_depth[2] = -3.0;
        assert!(
            bake_vol_factors(eye, &dirs, &bad_depth, &height, &params, &config, [4, 4]).is_err(),
            "negative depth fails"
        );
        let mut bad_dir = dirs.clone();
        bad_dir[5] = [0.0, 0.0, 0.0];
        assert!(
            bake_vol_factors(eye, &bad_dir, &depth, &height, &params, &config, [4, 4]).is_err(),
            "zero ray fails"
        );
    }

    #[test]
    fn vol_composite_holds_ramp() {
        let fog: [f32; 3] = [0.2, 0.3, 0.4];
        let base: [[f32; 3]; 2] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let out: Vec<[f32; 3]> = composite_vol(&base, &[0.0, 1.0], fog).expect("ramp mixes");
        assert_eq!(out[0], base[0], "clear factor keeps base");
        for c in 0..3 {
            assert!(
                (out[1][c] - fog[c]).abs() <= VOL_SKY_RAMP_TOL,
                "full factor holds the ramp at channel {c}"
            );
        }
        assert!(
            composite_vol(&base, &[0.5], fog).is_err(),
            "length mismatch fails"
        );
        assert_eq!(combine_factors(0.0, 0.5), 0.5, "clear lane keeps the other");
        assert_eq!(combine_factors(1.0, 0.5), 1.0, "opaque lane wins");
        assert_eq!(
            combine_factors(0.25, 0.25),
            0.4375,
            "quarter factors combine exact"
        );
    }

    #[test]
    fn vol_graph_orders_vol_before_forward() {
        let params = VolParams::day();
        let config = config_for_quality(Quality::Comfort);
        let plan: VolPassPlan = vol_pass_plan(&params, &config, [64, 64]).expect("plan sizes");
        assert_eq!(plan.vol_pass, VOL_PASS, "pass name rides the plan");
        assert_eq!(plan.vol_resource, VOL_RESOURCE, "resource rides the plan");
        assert_eq!(
            plan.vol_bytes,
            64u64 * 64u64 * VOL_BYTES_PER_TEXEL,
            "factor bytes are texels times four"
        );
        let compiled = compile_vol_for_forward(&params, &config, [64, 64]).expect("graph compiles");
        let names: Vec<&str> = compiled
            .order()
            .iter()
            .filter_map(|id| compiled.pass(*id).map(|p| p.name.as_str()))
            .collect();
        assert_eq!(names, [VOL_PASS, "forward"], "vol runs before forward");
        assert!(
            compile_vol_for_forward(&params, &config, [0, 64]).is_err(),
            "zero width fails the plan"
        );
    }

    #[test]
    fn vol_headless_seam_names_the_bind() {
        let params = VolParams::day();
        let config = config_for_quality(Quality::Comfort);
        let (dirs, depth, height) = flat_frame(16, 20.0, 5.0, [1.0, 0.0, 0.0]);
        let eye: [f32; 3] = [0.0, 5.0, 0.0];
        let (field, ms) = bake_vol_factors(eye, &dirs, &depth, &height, &params, &config, [4, 4])
            .expect("seam field bakes");
        match vol_headless_seam(&field, &look_frame_4()) {
            Err(crate::fog::FogError::NoDevice(detail)) => {
                assert!(
                    detail.contains("forward"),
                    "names the missing bind: {detail}"
                );
            }
            other => panic!("seam must name the forward bind, got {other:?}"),
        }
        let receipt = VolReceipt {
            frame_index: 7,
            vol_texels: u64::try_from(field.texels()).expect("texels fit u64"),
            ms,
        };
        assert_eq!(receipt.vol_budget_ms(), VOL_BUDGET_MS);
        assert_eq!(receipt.cloud_budget_ms(), CLOUD_BUDGET_MS);
        assert_eq!(receipt.budget_ms(), VOL_BUDGET_MS + CLOUD_BUDGET_MS);
        assert_eq!(
            receipt.within_budget(),
            ms <= VOL_BUDGET_MS + CLOUD_BUDGET_MS
        );
        let wide = crate::gtao::LookFrame {
            viewport_px: [8, 8],
            ..look_frame_4()
        };
        assert!(
            vol_headless_seam(&field, &wide).is_err(),
            "field/frame mismatch fails loudly"
        );
    }

    #[test]
    fn vol_root_hint_reads_the_hook() {
        assert_eq!(
            crate::volumetric_hint(Quality::Potato),
            (VOL_STEPS_MIN, false),
            "root hint carries potato savings"
        );
        assert_eq!(
            crate::volumetric_hint(Quality::High),
            (VOL_STEPS_MAX, true),
            "root hint carries top-tier march"
        );
    }

    #[test]
    fn vol_shader_packs_variants_and_guards() {
        for marker in [
            "vol_main",
            "vol_sky_main",
            "vol_hash",
            "0.0001",
            "workgroup_size(8, 8)",
        ] {
            assert!(VOL_WGSL.contains(marker), "embedded shader keeps {marker}");
        }
    }
}
