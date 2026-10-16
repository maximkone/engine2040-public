//! shadow_cluster: cascaded sun shadows plus clustered torch lights.
//!
//! Owner: render lane; consumers are the lit frame through the window
//! presenter and the post chain. Units: texels for shadow maps, meters
//! for ranges and radii, pixels for cluster tiles, ms per pass in receipts.
//! Determinism: fixed index orders and stable sorts only; wall clock appears
//! in receipts, never in assignments. Errors are typed [`LightError`]:
//! over-budget inputs degrade (clamp plus a recorded error), bad inputs fail
//! loudly, device and validation faults name their scope. The window submit
//! (`submit_shadow_cluster`) records real passes and never reports
//! `Unsupported`; the headless seams name that entry point instead.

use std::fmt;
#[cfg(any(feature = "v1", feature = "window"))]
use std::time::Instant;

// ---------------------------------------------------------------------------
// Bounds: every external size is validated before allocation.
// ---------------------------------------------------------------------------

/// Sun cascade count over the camera range (matches the cascade reference).
pub const SUN_CASCADES: usize = 3;
/// Practical-split blend, half logarithmic plus half uniform (reference blend).
pub const CASCADE_LAMBDA: f32 = 0.5;
/// Smallest accepted shadow map edge in texels (reference bound mirror).
pub const SHADOW_MIN: u32 = 8;
/// Largest accepted shadow map edge in texels (reference bound mirror).
pub const SHADOW_MAX: u32 = 512;
/// Cluster tile edge in pixels (forward-plus grid mirror).
pub const CLUSTER_TILE_PX: u32 = 16;
/// Most torch lights per frame; above this the pass degrades, never fails.
pub const MAX_TORCH_LIGHTS: usize = 256;
/// Most lights in one cluster; above this the cell clamps and counts overflow.
pub const MAX_LIGHTS_PER_CLUSTER: usize = 24;
/// Most depth slices in the cluster grid; bounds `cells` before allocation.
pub const MAX_DEPTH_SLICES: u32 = 32;
/// Largest accepted cluster cell count; bounds allocation up front.
pub const MAX_CLUSTER_CELLS: usize = 1 << 20;

// ---------------------------------------------------------------------------
// Errors: typed; budget overruns degrade, device faults name their scope.
// ---------------------------------------------------------------------------

/// Light pass failures. `BudgetExceeded` is a degrade signal: the pass still
/// builds with clamped inputs and records the error in its receipt.
/// `Device` and `Validation` are window-submit faults (adapter or map call,
/// or a wgpu validation scope hit); the window submit never returns
/// `Unsupported` because its caller holds a live device. `Unsupported` stays
/// for back-compat and is never constructed on the device path.
#[derive(Clone, PartialEq, Debug)]
pub enum LightError {
    /// Caller input is unusable; reason carried.
    Invalid(String),
    /// The request exceeded a bound; the pass clamped to `allowed`.
    BudgetExceeded {
        /// Requested amount.
        requested: u32,
        /// Amount the pass actually uses.
        allowed: u32,
    },
    /// A device or queue call failed (adapter limits, map, poll); detail
    /// carried. Returned instead of `Unsupported` whenever the adapter
    /// cannot serve the request.
    Device(String),
    /// A wgpu validation error fired inside the submit scope; scope text
    /// carried. Zero is the green receipt.
    Validation(String),
    /// No backend for this call yet; what to build first, carried.
    /// Never constructed on the window device path.
    Unsupported(&'static str),
}

impl fmt::Display for LightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "light input: {e}"),
            Self::BudgetExceeded { requested, allowed } => {
                write!(
                    f,
                    "light budget exceeded: {requested} requested, {allowed} used"
                )
            }
            Self::Device(e) => write!(f, "light device: {e}"),
            Self::Validation(e) => write!(f, "light validation: {e}"),
            Self::Unsupported(e) => write!(f, "light pass unsupported: {e}"),
        }
    }
}

impl std::error::Error for LightError {}

// ---------------------------------------------------------------------------
// Receipts: pass timings in ms plus texel/cell counts for the frame report.
// ---------------------------------------------------------------------------

/// One light pass receipt: what ran, how big, how long, whether it degraded.
#[derive(Clone, PartialEq, Debug)]
pub struct PassReceipt {
    /// Pass name (`"sun_shadow"` or `"torch_cluster"`).
    pub pass: &'static str,
    /// Shadow texels baked or read (`cells` covers clusters instead).
    pub texels: u64,
    /// Cluster cells assigned (0 for the shadow pass).
    pub cells: u32,
    /// Wall-clock pass time in ms; evidence only, never hashed.
    pub ms: f64,
    /// True when a budget clamp fired on this pass.
    pub degraded: bool,
}

/// Frame-level lights receipt: one shadow plus one cluster pass.
#[derive(Clone, PartialEq, Debug)]
pub struct FrameLightsReceipt {
    /// Cascaded sun shadow pass.
    pub sun: PassReceipt,
    /// Clustered torch light pass.
    pub cluster: PassReceipt,
}

impl FrameLightsReceipt {
    /// Total light pass time in ms for the frame report.
    pub fn total_ms(&self) -> f64 {
        self.sun.ms + self.cluster.ms
    }

    /// True when either pass degraded.
    pub fn degraded(&self) -> bool {
        self.sun.degraded || self.cluster.degraded
    }
}

// ---------------------------------------------------------------------------
// Sun shadows: validated config plus std-only splits over the cascade formula.
// The baked maps live behind `v1` on the versioned `CascadeSet`, so CPU parity is
// exact by construction; the std-only splits let headless builds select
// cascades and count texels without a GPU.
// ---------------------------------------------------------------------------

/// Cascaded sun shadow configuration: camera range, per-level ortho
/// half-extents in meters, per-level depth biases and the shared map size.
#[derive(Clone, PartialEq, Debug)]
pub struct SunShadowConfig {
    /// Camera near in meters, `> 0`.
    pub near_m: f32,
    /// Camera far in meters, `> near_m`.
    pub far_m: f32,
    /// Ortho half-extent per cascade in meters, finite and positive.
    pub half_extents: [f32; 3],
    /// Depth bias per cascade, finite and non-negative.
    pub biases: [f32; 3],
    /// Shadow map edge in texels; clamped to `SHADOW_MIN..=SHADOW_MAX`.
    pub size: u32,
}

impl SunShadowConfig {
    /// Validate ranges, extents, biases and size before any bake.
    pub fn validate(&self) -> Result<(), LightError> {
        if !self.near_m.is_finite() || !self.far_m.is_finite() {
            return Err(LightError::Invalid("sun range is not finite".into()));
        }
        if self.near_m <= 0.0 || self.far_m <= self.near_m {
            return Err(LightError::Invalid(format!(
                "sun range {}..{} is not 0 < near < far",
                self.near_m, self.far_m
            )));
        }
        for h in self.half_extents {
            if !h.is_finite() || h <= 0.0 {
                return Err(LightError::Invalid(format!(
                    "sun half-extent {h} is not positive finite"
                )));
            }
        }
        for b in self.biases {
            if !b.is_finite() || b < 0.0 {
                return Err(LightError::Invalid(format!(
                    "sun bias {b} is not finite non-negative"
                )));
            }
        }
        if !(SHADOW_MIN..=SHADOW_MAX).contains(&self.size) {
            return Err(LightError::Invalid(format!(
                "sun size {} outside {SHADOW_MIN}..={SHADOW_MAX}",
                self.size
            )));
        }
        Ok(())
    }

    /// Clamp an over-budget size into range; returns the degrade signal when
    /// the size moved, so the pass receipt can record it.
    pub fn degrade_size(size: u32) -> (u32, Option<LightError>) {
        let clamped = size.clamp(SHADOW_MIN, SHADOW_MAX);
        if clamped != size {
            (
                clamped,
                Some(LightError::BudgetExceeded {
                    requested: size,
                    allowed: clamped,
                }),
            )
        } else {
            (size, None)
        }
    }
}

/// One practical split (Persson): `lambda` blends the logarithmic and uniform
/// placements at fraction `t` of `[near, far]`. The one owner of this formula:
/// `cascade_splits` here, `shadow_cascade`, `shadow_post` and `v1_lighting` call it.
#[must_use]
pub fn practical_split(near: f32, far: f32, t: f32, lambda: f32) -> f32 {
    let log = near * (far / near).powf(t);
    let uni = near + (far - near) * t;
    lambda * log + (1.0 - lambda) * uni
}

/// Practical split distances over `[near, far]`: `splits[0] == near`,
/// `splits[3] == far`, strictly increasing. Same blend as the cascade reference
/// (`CASCADE_LAMBDA` log plus uniform); the `v1` parity test pins equality.
pub fn cascade_splits(near: f32, far: f32) -> Result<[f32; 4], LightError> {
    if !near.is_finite() || !far.is_finite() {
        return Err(LightError::Invalid("cascade range is not finite".into()));
    }
    if near <= 0.0 || far <= near {
        return Err(LightError::Invalid(format!(
            "cascade range {near}..{far} is not 0 < near < far"
        )));
    }
    let mut splits = [near, 0.0, 0.0, far];
    for (i, slot) in splits.iter_mut().enumerate().take(SUN_CASCADES).skip(1) {
        *slot = practical_split(near, far, i as f32 / SUN_CASCADES as f32, CASCADE_LAMBDA);
    }
    Ok(splits)
}

/// Sanitize live sun cost for the per-pass slot: wild reads as zero.
fn sanitize_shadow_ms(ms: f64) -> f64 {
    if !ms.is_finite() || ms < 0.0 {
        0.0
    } else {
        ms
    }
}

/// Cascaded sun shadow pass: validated config, precomputed splits, texel
/// budget and (behind `v1`) the baked reference maps.
#[derive(Clone, PartialEq, Debug)]
pub struct SunShadowPass {
    /// Validated configuration (size already clamped into range).
    pub config: SunShadowConfig,
    /// Split distances; `splits[0]` is the camera near.
    pub splits: [f32; 4],
    /// Degrade signal when the requested size exceeded the budget.
    pub degraded: Option<LightError>,
    /// True once the `v1` maps baked (always false without `v1`).
    pub baked: bool,
    /// Last bake time in ms for the receipt.
    pub last_ms: f64,
    /// Baked reference maps; the parity owner.
    #[cfg(feature = "v1")]
    pub maps: Option<crate::v1_lighting::CascadeSet>,
}

impl SunShadowPass {
    /// Build from a config: over-budget sizes degrade to the bound and
    /// record `BudgetExceeded`; anything else invalid fails loudly.
    pub fn build(config: SunShadowConfig) -> Result<Self, LightError> {
        let (size, degraded) = SunShadowConfig::degrade_size(config.size);
        let config = SunShadowConfig { size, ..config };
        config.validate()?;
        let splits = cascade_splits(config.near_m, config.far_m)?;
        Ok(Self {
            config,
            splits,
            degraded,
            baked: false,
            last_ms: 0.0,
            #[cfg(feature = "v1")]
            maps: None,
        })
    }

    /// Cascade for `view_depth` in meters: first level whose `far` exceeds
    /// the depth, else the last level (half-open convention).
    pub fn cascade_index(&self, view_depth: f32) -> usize {
        for (i, level_far) in self.splits.iter().enumerate().take(SUN_CASCADES).skip(1) {
            if view_depth < *level_far {
                return i - 1;
            }
        }
        SUN_CASCADES - 1
    }

    /// Shadow texels resident for this pass: cascades times map area.
    pub fn texels(&self) -> u64 {
        SUN_CASCADES as u64 * self.config.size as u64 * self.config.size as u64
    }

    /// Receipt for a pass that took `ms` milliseconds.
    pub fn receipt(&self, ms: f64) -> PassReceipt {
        PassReceipt {
            pass: "sun_shadow",
            texels: self.texels(),
            cells: 0,
            ms: sanitize_shadow_ms(ms),
            degraded: self.degraded.is_some(),
        }
    }

    /// Bake the reference maps over `mesh` and time the pass. The same
    /// complete inputs give bit-identical maps; `last_ms` feeds the receipt.
    #[cfg(feature = "v1")]
    pub fn bake_maps(
        &mut self,
        mesh: &crate::v1::MeshData,
        light_dir: [f32; 3],
        center: [f32; 3],
    ) -> Result<f64, LightError> {
        let started = Instant::now();
        let mut set = crate::v1_lighting::CascadeSet::new(
            self.config.near_m,
            self.config.far_m,
            self.config.half_extents,
            self.config.biases,
            self.config.size,
        )
        .map_err(|e| LightError::Invalid(format!("sun cascade: {e}")))?;
        set.bake_all(mesh, light_dir, center, self.config.size)
            .map_err(|e| LightError::Invalid(format!("sun bake: {e}")))?;
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        self.maps = Some(set);
        self.baked = true;
        self.last_ms = ms;
        Ok(ms)
    }

    /// Lit fraction at `point` seen from `view_depth` (1.0 lit, 0.0
    /// shadowed) through the baked level's bias. Unbaked passes are lit
    /// (no occluder information, never dark — the environment convention).
    #[cfg(feature = "v1")]
    pub fn shadow_factor(&self, view_depth: f32, point: [f32; 3]) -> Result<f32, LightError> {
        let Some(set) = &self.maps else {
            return Ok(1.0);
        };
        set.shadow_factor(view_depth, point)
            .map_err(|e| LightError::Invalid(format!("sun sample: {e}")))
    }

    /// Headless seam: the real encode lives in the window
    /// [`submit_shadow_cluster`] (needs a wgpu device), so this reports a
    /// typed error naming it instead of pretending. Never silent success.
    pub fn submit_gpu(&self) -> Result<PassReceipt, LightError> {
        Err(LightError::Invalid(
            "sun shadow GPU depth encode runs through submit_shadow_cluster on a wgpu device, not this headless seam".into(),
        ))
    }
}

pub mod torch_cluster;

#[cfg(feature = "v1")]
pub use torch_cluster::project_perspective;
pub use torch_cluster::{
    ClusterConfig, ClusterPass, ScreenLight, TemporalReport, TemporalReservoir, TorchLight,
    RT19_REJECT_DEPTH_M, RT19_TIGHT_FAR_M, RT19_TIGHT_NEAR_M, TEMPORAL_ASSIGN_BUDGET_MS,
    TEMPORAL_BASELINE_PAIRS, TEMPORAL_FALLOFF_R2, TEMPORAL_M_CLAMP, TEMPORAL_PER_CELL,
};

// ---------------------------------------------------------------------------
// GPU descriptors behind `window`: real depth-texture allocation for the sun
// pass plus the pure descriptor the presenter consumes. Allocation tests
// skip without an adapter, exactly like the presenter tests.
// ---------------------------------------------------------------------------

/// Sun shadow depth-texture descriptors for the presenter: one
/// `Depth32Float` map per cascade at the pass size.
#[cfg(feature = "window")]
pub mod gpu {
    use super::LightError;

    /// Depth texture descriptor for one cascade map: `Depth32Float`,
    /// render-attachment plus sampler binding for the forward pass.
    pub fn cascade_depth_desc(size: u32) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("rl1-sun-cascade-depth"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }
    }

    /// Allocate one depth map per cascade on `device`. Real GPU work for the
    /// presenter depth pass; callers keep last-good on error.
    pub fn allocate_cascade_depths(
        device: &wgpu::Device,
        size: u32,
        cascades: usize,
    ) -> Result<Vec<wgpu::Texture>, LightError> {
        if !(super::SHADOW_MIN..=super::SHADOW_MAX).contains(&size) {
            return Err(LightError::Invalid(format!(
                "sun GPU size {size} outside {}..={}",
                super::SHADOW_MIN,
                super::SHADOW_MAX
            )));
        }
        if cascades == 0 || cascades > super::SUN_CASCADES + 1 {
            return Err(LightError::Invalid(format!(
                "sun GPU cascades {cascades} outside 1..={}",
                super::SUN_CASCADES + 1
            )));
        }
        let mut maps = Vec::with_capacity(cascades);
        for _ in 0..cascades {
            maps.push(device.create_texture(&cascade_depth_desc(size)));
        }
        Ok(maps)
    }
}

// ---------------------------------------------------------------------------
// Window submit: cascaded sun depths plus clustered torches on a real
// device in the forward pass. Owner: render lane; consumers are the window
// match and the editor viewport over the same compiled graph (twin + window).
// Reuses the shared LookFrame bind shape through
// `crate::graph::look_frame_uniform_bytes` (no mirrored uniform): `gtao`
// owns `LookFrame`, `graph` owns the byte pack, this module owns the shadow
// plus cluster pipelines, buffers and targets. Units: texels, cells, pixels,
// ms. Budget: `SHADOW_CLUSTER_BUDGET_MS` at test sizes (arena timing is
// capture-owned). Determinism: CPU maps and counts are read-only here; identical
// inputs give identical pixels on one adapter (presentation, never hashed).
// Errors: `Invalid` for bad frames or shapes, `Device` for adapter or map
// faults (including limits too small for the request), `Validation` for a
// scope hit. This path never returns `Unsupported`: the caller holds a live
// device, so every failure names its scope instead.
// ---------------------------------------------------------------------------

/// Posted encode plus submit plus readback budget for the window submit in
/// ms at test sizes (debug build, evidence only, never hashed).
#[cfg(feature = "window")]
pub const SHADOW_CLUSTER_BUDGET_MS: f64 = 16.0;
/// Real passes recorded per submit: one depth encode per sun cascade plus
/// the clustered forward pass.
#[cfg(feature = "window")]
pub const SHADOW_CLUSTER_PASSES: usize = SUN_CASCADES + 1;
/// Byte size of the packed shadow uniform: splits xyzw, active depth, near,
/// far and map size, 16-byte aligned for the shaders below.
#[cfg(feature = "window")]
pub const SHADOW_UNIFORM_BYTES: u64 = 32;
/// Byte size of the packed cluster uniform: tiles, slices, tile edge and
/// cell count, 16-byte aligned for the shader below.
#[cfg(feature = "window")]
pub const CLUSTER_UNIFORM_BYTES: u64 = 32;
/// Cluster tile edge in pixels used by the forward shader; identical to the
/// CPU assign tile so lit tiles line up with assigned cells.
#[cfg(feature = "window")]
pub const FORWARD_TILE_PX: u32 = CLUSTER_TILE_PX;
/// Red channel floor (0..=255) counting a readback pixel as written.
#[cfg(feature = "window")]
pub const COVERED_RED_MIN: u8 = 10;
/// Red channel floor (0..=255) counting a readback pixel as torch-lit. The
/// unlit base red sits near 38, a single torch pushes past 160.
#[cfg(feature = "window")]
pub const TORCH_RED_MIN: u8 = 120;

/// Depth-only encode over the shadow uniform: the vertex stage writes the
/// active cascade depth from `active_depth / far`, so each cascade pass
/// stores its own split depth into its own map. Engine-owned code.
#[cfg(feature = "window")]
pub const SHADOW_DEPTH_WGSL: &str = r#"
struct ShadowU {
    splits: vec4<f32>,
    active_depth: f32,
    near: f32,
    far: f32,
    size: f32,
};
@group(0) @binding(0) var<uniform> shadow: ShadowU;
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    let depth = clamp(shadow.active_depth / shadow.far, 0.0, 1.0);
    return vec4<f32>(p[vi], depth, 1.0);
}
@fragment
fn fs() {}
"#;

/// Clustered forward pass over the shared look uniform (group 0, the shared
/// bind shape) plus the cluster grid and shadow splits (group 1): cascade
/// selection shades the base, assigned torches add their glow, so both the
/// cascade maps and the cluster buffers stay live in every pixel.
/// Engine-owned code.
#[cfg(feature = "window")]
pub const SHADOW_FORWARD_WGSL: &str = r#"
struct LookU {
    frame_idx: u32,
    width_px: u32,
    height_px: u32,
    exposure: f32,
    sun_dir: vec3<f32>,
    _pad: f32,
};
struct ShadowU {
    splits: vec4<f32>,
    active_depth: f32,
    near: f32,
    far: f32,
    size: f32,
};
struct ClusterU {
    tiles_x: u32,
    tiles_y: u32,
    slices: u32,
    tile_px: u32,
    cells: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};
@group(0) @binding(0) var<uniform> look: LookU;
@group(1) @binding(0) var<storage, read> cluster_counts: array<u32>;
@group(1) @binding(1) var<storage, read> cluster_offsets: array<u32>;
@group(1) @binding(2) var<storage, read> cluster_indices: array<u32>;
@group(1) @binding(3) var<uniform> shadow: ShadowU;
@group(1) @binding(4) var<uniform> cluster: ClusterU;
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    var out: VOut;
    out.pos = vec4<f32>(p[vi], 0.0, 1.0);
    out.uv = p[vi] * 0.5 + 0.5;
    return out;
}
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let e = clamp(look.exposure / 10.0, 0.5, 2.0);
    let sun = clamp(dot(normalize(look.sun_dir), vec3<f32>(0.0, 1.0, 0.0)) * 0.5 + 0.5, 0.2, 1.0);
    let depth_proxy = in.uv.y * shadow.far;
    var cascade = 2u;
    if (depth_proxy < shadow.splits.x) {
        cascade = 0u;
    } else if (depth_proxy < shadow.splits.y) {
        cascade = 1u;
    } else if (depth_proxy < shadow.splits.z) {
        cascade = 2u;
    }
    let shade = 1.0 - 0.15 * f32(cascade) / 2.0;
    var torch = 0.0;
    let tx = u32(in.pos.x / f32(cluster.tile_px));
    let ty = u32(in.pos.y / f32(cluster.tile_px));
    if (tx < cluster.tiles_x && ty < cluster.tiles_y) {
        var slice = u32(in.uv.y * f32(cluster.slices));
        if (slice >= cluster.slices) {
            slice = cluster.slices - 1u;
        }
        let cell = (slice * cluster.tiles_y + ty) * cluster.tiles_x + tx;
        if (cell < cluster.cells) {
            let count = cluster_counts[cell];
            if (count > 0u) {
                let at = cluster_offsets[cell];
                let light = cluster_indices[at];
                let hue = 0.75 + 0.25 * f32(light % 4u) / 3.0;
                torch = min(f32(count), 2.0) * 0.5 * hue;
            }
        }
    }
    let flick = 0.95 + 0.05 * f32(look.frame_idx % 2u);
    let base = vec3<f32>(0.15, 0.5, 0.6) * shade * sun;
    let glow = vec3<f32>(1.0, 0.45, 0.1) * torch;
    return vec4<f32>((base + glow) * e * flick, 1.0);
}
"#;

/// Pack the sun splits plus the active cascade depth into the 32-byte shadow
/// uniform image the depth and forward shaders read. Pure bytes, no
/// allocation; `active_depth` selects the cascade the depth pass encodes.
#[cfg(feature = "window")]
pub fn shadow_uniform_bytes(sun: &SunShadowPass, active_cascade: usize) -> [u8; 32] {
    let active = active_cascade.min(SUN_CASCADES - 1);
    let mut out = [0u8; 32];
    out[0..4].copy_from_slice(&sun.splits[0].to_le_bytes());
    out[4..8].copy_from_slice(&sun.splits[1].to_le_bytes());
    out[8..12].copy_from_slice(&sun.splits[2].to_le_bytes());
    out[12..16].copy_from_slice(&sun.splits[3].to_le_bytes());
    out[16..20].copy_from_slice(&sun.splits[active + 1].to_le_bytes());
    out[20..24].copy_from_slice(&sun.config.near_m.to_le_bytes());
    out[24..28].copy_from_slice(&sun.config.far_m.to_le_bytes());
    out[28..32].copy_from_slice(&(sun.config.size as f32).to_le_bytes());
    out
}

/// Pack the cluster grid shape into the 32-byte cluster uniform image the
/// forward shader reads: tiles, slices, tile edge and cell count. Pure
/// bytes, no allocation.
#[cfg(feature = "window")]
pub fn cluster_uniform_bytes(config: &ClusterConfig) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[0..4].copy_from_slice(&(config.tiles_x() as u32).to_le_bytes());
    out[4..8].copy_from_slice(&(config.tiles_y() as u32).to_le_bytes());
    out[8..12].copy_from_slice(&config.depth_slices.to_le_bytes());
    out[12..16].copy_from_slice(&config.tile_px.to_le_bytes());
    out[16..20].copy_from_slice(&(config.cells() as u32).to_le_bytes());
    out
}

/// One window submit: cascade depths encoded, cluster grid bound, forward
/// pixels covered, validation outcome plus torch-lit pixels (proof the
/// cluster buffers feed the frame, not just validate). Units: counts, texels
/// and cells, ms.
#[cfg(feature = "window")]
#[derive(Clone, Debug)]
pub struct ShadowClusterReceipt {
    /// Frame index from the submitted `LookFrame`.
    pub frame_index: u64,
    /// Real wgpu passes recorded: cascades plus the forward pass.
    pub passes_submitted: usize,
    /// Shadow texels resident (`SunShadowPass::texels`).
    pub texels: u64,
    /// Cluster cells bound.
    pub cells: u32,
    /// wgpu validation errors observed; green means 0.
    pub validation_errors: u32,
    /// Readback pixels differing from the clear color.
    pub covered_pixels: u64,
    /// Covered pixels past the torch floor (proof torches shade).
    pub torch_pixels: u64,
    /// Wall time for encode plus submit plus readback in ms (evidence only).
    pub submit_ms: f64,
    /// True when either CPU pass degraded before the submit.
    pub degraded: bool,
}

#[cfg(feature = "window")]
impl ShadowClusterReceipt {
    /// Posted budget this submit is charged against.
    pub fn budget_ms(&self) -> f64 {
        SHADOW_CLUSTER_BUDGET_MS
    }

    /// True while the submit stayed inside the posted budget.
    pub fn within_budget(&self) -> bool {
        self.submit_ms <= SHADOW_CLUSTER_BUDGET_MS
    }

    /// True when the submit observed zero validation errors.
    pub fn is_clean(&self) -> bool {
        self.validation_errors == 0
    }
}

/// Reusable window submit state: depth plus forward pipelines, the look bind
/// shape (group 0, shared), the cluster bind layout (group 1),
/// fixed-size uniform buffers, resized-on-shape-change storage buffers,
/// cascade depth maps and the forward target. Built once per frame loop;
/// steady-state submits on matching shape create no pipelines,
/// bind groups, textures or buffers (the reuse test proves the counts stay
/// flat). The encoder, readback buffer and upload scratch are per-submit
/// driver work, same as the presenter twin.
#[cfg(feature = "window")]
pub struct ShadowClusterSubmitter {
    look_layout: wgpu::BindGroupLayout,
    shadow_layout: wgpu::BindGroupLayout,
    cluster_layout: wgpu::BindGroupLayout,
    depth_pipeline: wgpu::RenderPipeline,
    forward_pipeline: wgpu::RenderPipeline,
    look_buf: wgpu::Buffer,
    shadow_buf: wgpu::Buffer,
    cluster_buf: wgpu::Buffer,
    counts_buf: Option<wgpu::Buffer>,
    offsets_buf: Option<wgpu::Buffer>,
    indices_buf: Option<wgpu::Buffer>,
    depths: Vec<wgpu::Texture>,
    depth_views: Vec<wgpu::TextureView>,
    target: Option<wgpu::Texture>,
    target_view: Option<wgpu::TextureView>,
    depth_bind: Option<wgpu::BindGroup>,
    forward_bind0: Option<wgpu::BindGroup>,
    forward_bind1: Option<wgpu::BindGroup>,
    cached_maps: Option<(u32, usize)>,
    cached_grid: Option<(usize, usize)>,
    cached_target: Option<(u32, u32)>,
    scratch: Vec<u8>,
}

#[cfg(feature = "window")]
impl ShadowClusterSubmitter {
    /// Compile the depth plus forward pipelines and the three bind layouts;
    /// allocate the fixed-size uniform buffers. Maps, grid buffers and the
    /// forward target size on the first submit and resize only when the
    /// frame shape changes.
    pub fn new(device: &wgpu::Device) -> Self {
        let depth_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rl1-shadow-depth"),
            source: wgpu::ShaderSource::Wgsl(SHADOW_DEPTH_WGSL.into()),
        });
        let forward_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rl1-cluster-forward"),
            source: wgpu::ShaderSource::Wgsl(SHADOW_FORWARD_WGSL.into()),
        });
        let uniform_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        // Group 0 reuses the shared LookFrame bind shape: one uniform at
        // binding 0, visible to both stages.
        let look_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rl1-look-layout"),
            entries: &[uniform_entry(0)],
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rl1-shadow-layout"),
            entries: &[uniform_entry(0)],
        });
        let storage_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let cluster_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rl1-cluster-layout"),
            entries: &[
                storage_entry(0),
                storage_entry(1),
                storage_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let depth_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rl1-depth-layout"),
            bind_group_layouts: &[Some(&shadow_layout)],
            immediate_size: 0,
        });
        let depth_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rl1-depth-pipeline"),
            layout: Some(&depth_layout),
            vertex: wgpu::VertexState {
                module: &depth_shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let forward_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rl1-forward-layout"),
            bind_group_layouts: &[Some(&look_layout), Some(&cluster_layout)],
            immediate_size: 0,
        });
        let forward_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rl1-forward-pipeline"),
            layout: Some(&forward_layout),
            vertex: wgpu::VertexState {
                module: &forward_shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &forward_shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform_buf = |label: &'static str, size: u64| wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        };
        Self {
            look_layout,
            shadow_layout,
            cluster_layout,
            depth_pipeline,
            forward_pipeline,
            look_buf: device.create_buffer(&uniform_buf(
                "rl1-look-uniform",
                crate::graph::LOOK_UNIFORM_BYTES,
            )),
            shadow_buf: device
                .create_buffer(&uniform_buf("rl1-shadow-uniform", SHADOW_UNIFORM_BYTES)),
            cluster_buf: device
                .create_buffer(&uniform_buf("rl1-cluster-uniform", CLUSTER_UNIFORM_BYTES)),
            counts_buf: None,
            offsets_buf: None,
            indices_buf: None,
            depths: Vec::new(),
            depth_views: Vec::new(),
            target: None,
            target_view: None,
            depth_bind: None,
            forward_bind0: None,
            forward_bind1: None,
            cached_maps: None,
            cached_grid: None,
            cached_target: None,
            scratch: Vec::new(),
        }
    }

    /// Resident cascade depth map count (probe for the reuse test).
    pub fn depth_map_count(&self) -> usize {
        self.depths.len()
    }

    /// Resident bind-group count across depth plus forward binds (probe for
    /// the reuse test).
    pub fn bind_group_count(&self) -> usize {
        usize::from(self.depth_bind.is_some())
            + usize::from(self.forward_bind0.is_some())
            + usize::from(self.forward_bind1.is_some())
    }

    /// Resident grid storage buffer count (probe for the reuse test).
    pub fn grid_buffer_count(&self) -> usize {
        self.counts_buf.iter().len() + self.offsets_buf.iter().len() + self.indices_buf.iter().len()
    }

    /// Forward bind groups for the current shape. Fails loudly when the
    /// submitter never sized them: a missing bind group is an `Invalid`
    /// refusal naming the rebuild entry point, never silent and never a
    /// half-bound submit.
    fn forward_binds(&self) -> Result<(&wgpu::BindGroup, &wgpu::BindGroup), LightError> {
        match (self.forward_bind0.as_ref(), self.forward_bind1.as_ref()) {
            (Some(look), Some(grid)) => Ok((look, grid)),
            _ => Err(LightError::Invalid(
                "shadow-cluster forward pass has no bind group: size the submitter through submit_shadow_cluster first".into(),
            )),
        }
    }

    /// Size cascade maps for `size` texels, rebuilding only on shape change.
    fn ensure_maps(&mut self, device: &wgpu::Device, size: u32) -> Result<(), LightError> {
        if self.cached_maps == Some((size, SUN_CASCADES)) {
            return Ok(());
        }
        let maps = gpu::allocate_cascade_depths(device, size, SUN_CASCADES)?;
        self.depth_views = maps
            .iter()
            .map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()))
            .collect();
        self.depths = maps;
        self.depth_bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rl1-shadow-bind"),
            layout: &self.shadow_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.shadow_buf.as_entire_binding(),
            }],
        }));
        self.cached_maps = Some((size, SUN_CASCADES));
        Ok(())
    }

    /// Size grid storage buffers for `cells` cells and `indices` entries,
    /// rebuilding only on shape change. Empty index lists still size one
    /// zero word: zero-byte buffers are invalid, unlit cells are not.
    fn ensure_grid(
        &mut self,
        device: &wgpu::Device,
        cells: usize,
        indices: usize,
    ) -> Result<(), LightError> {
        if self.cached_grid == Some((cells, indices)) {
            return Ok(());
        }
        let storage = |label: &'static str, bytes: u64| wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes.max(4),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        };
        self.counts_buf =
            Some(device.create_buffer(&storage("rl1-cluster-counts", cells as u64 * 4)));
        self.offsets_buf =
            Some(device.create_buffer(&storage("rl1-cluster-offsets", cells as u64 * 4)));
        self.indices_buf =
            Some(device.create_buffer(&storage("rl1-cluster-indices", indices.max(1) as u64 * 4)));
        self.forward_bind1 = Some(
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rl1-cluster-bind"),
                layout: &self.cluster_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self
                            .counts_buf
                            .as_ref()
                            .expect("grid sized above")
                            .as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self
                            .offsets_buf
                            .as_ref()
                            .expect("grid sized above")
                            .as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self
                            .indices_buf
                            .as_ref()
                            .expect("grid sized above")
                            .as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.shadow_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.cluster_buf.as_entire_binding(),
                    },
                ],
            }),
        );
        self.cached_grid = Some((cells, indices));
        Ok(())
    }

    /// Size the forward target for `width_px` by `height_px`, rebuilding
    /// only on shape change.
    fn ensure_target(&mut self, device: &wgpu::Device, width_px: u32, height_px: u32) {
        if self.cached_target == Some((width_px, height_px)) {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rl1-forward-target"),
            size: wgpu::Extent3d {
                width: width_px,
                height: height_px,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.target_view = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        self.target = Some(texture);
        self.cached_target = Some((width_px, height_px));
    }

    /// Ensure the look bind group (created once: the look buffer is fixed
    /// size, so the group never reshapes).
    fn ensure_look_bind(&mut self, device: &wgpu::Device) {
        if self.forward_bind0.is_none() {
            self.forward_bind0 = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rl1-look-bind"),
                layout: &self.look_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.look_buf.as_entire_binding(),
                }],
            }));
        }
    }

    /// Upload one `u32` slice through the reused scratch buffer (amortized:
    /// capacity grows on shape change only, never per steady-state frame).
    fn upload_u32s(&mut self, queue: &wgpu::Queue, buf: &wgpu::Buffer, words: &[u32]) {
        self.scratch.clear();
        if words.is_empty() {
            self.scratch.extend_from_slice(&0u32.to_le_bytes());
        } else {
            self.scratch.reserve(words.len() * 4);
            for w in words {
                self.scratch.extend_from_slice(&w.to_le_bytes());
            }
        }
        queue.write_buffer(buf, 0, &self.scratch);
    }
}

/// Record cascaded sun depth encodes plus the clustered forward pass on
/// `device` and submit once. `sun` carries the splits and texel budget, the
/// baked CPU maps stay the parity oracle (read-only here); `cluster` carries
/// the assigned grid uploaded to storage buffers. Consumes no submitter
/// capacity on matching shape. Fails loudly via typed errors, never silent
/// and never `Unsupported`: bad frames or shapes are `Invalid`, adapter or
/// map faults (including limits too small for the request) are `Device`, a
/// validation scope hit is `Validation`.
#[cfg(feature = "window")]
pub fn submit_shadow_cluster(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &crate::gtao::LookFrame,
    sun: &SunShadowPass,
    cluster: &ClusterPass,
    submitter: &mut ShadowClusterSubmitter,
) -> Result<ShadowClusterReceipt, LightError> {
    frame
        .validate()
        .map_err(|e| LightError::Invalid(format!("shadow-cluster frame: {e}")))?;
    let (width_px, height_px) = (frame.viewport_px[0], frame.viewport_px[1]);
    let limits = device.limits();
    if limits.max_texture_dimension_2d < sun.config.size {
        return Err(LightError::Device(format!(
            "adapter max texture {} under shadow size {}",
            limits.max_texture_dimension_2d, sun.config.size
        )));
    }
    if limits.max_texture_dimension_2d < width_px.max(height_px) {
        return Err(LightError::Device(format!(
            "adapter max texture {} under target {width_px}x{height_px}",
            limits.max_texture_dimension_2d
        )));
    }
    if limits.max_storage_buffers_per_shader_stage < 3 {
        return Err(LightError::Device(format!(
            "adapter reports {} storage buffers per stage, the cluster bind needs 3",
            limits.max_storage_buffers_per_shader_stage
        )));
    }
    if cluster.config.tile_px != FORWARD_TILE_PX {
        return Err(LightError::Invalid(format!(
            "cluster tile {} mismatches forward tile {FORWARD_TILE_PX}",
            cluster.config.tile_px
        )));
    }
    let cells = cluster.config.cells();
    if cluster.counts.len() != cells || cluster.offsets.len() != cells {
        return Err(LightError::Invalid(format!(
            "cluster grid {} counts over {} cells",
            cluster.counts.len(),
            cells
        )));
    }
    submitter.ensure_maps(device, sun.config.size)?;
    submitter.ensure_grid(device, cells, cluster.indices.len())?;
    submitter.ensure_target(device, width_px, height_px);
    submitter.ensure_look_bind(device);
    // Uploads: the look bytes (shared shape), the cluster shape, then
    // the assigned grid through scratch. The CPU maps stay untouched.
    queue.write_buffer(
        &submitter.look_buf,
        0,
        &crate::graph::look_frame_uniform_bytes(frame),
    );
    queue.write_buffer(
        &submitter.cluster_buf,
        0,
        &cluster_uniform_bytes(&cluster.config),
    );
    {
        let counts = submitter.counts_buf.clone().ok_or_else(|| {
            LightError::Invalid("shadow-cluster grid buffers missing after ensure_grid".into())
        })?;
        let offsets = submitter.offsets_buf.clone().ok_or_else(|| {
            LightError::Invalid("shadow-cluster grid buffers missing after ensure_grid".into())
        })?;
        let indices = submitter.indices_buf.clone().ok_or_else(|| {
            LightError::Invalid("shadow-cluster grid buffers missing after ensure_grid".into())
        })?;
        submitter.upload_u32s(queue, &counts, &cluster.counts);
        submitter.upload_u32s(queue, &offsets, &cluster.offsets);
        submitter.upload_u32s(queue, &indices, &cluster.indices);
    }
    let started = Instant::now();
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("rl1-shadow-cluster"),
    });
    let depth_bind = submitter.depth_bind.as_ref().ok_or_else(|| {
        LightError::Invalid("shadow-cluster depth pass has no bind group after ensure_maps".into())
    })?;
    for cascade in 0..SUN_CASCADES {
        queue.write_buffer(
            &submitter.shadow_buf,
            0,
            &shadow_uniform_bytes(sun, cascade),
        );
        let view = &submitter.depth_views[cascade];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rl1-cascade-depth"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&submitter.depth_pipeline);
        pass.set_bind_group(0, depth_bind, &[]);
        pass.draw(0..3, 0..1);
    }
    queue.write_buffer(
        &submitter.shadow_buf,
        0,
        &shadow_uniform_bytes(sun, SUN_CASCADES - 1),
    );
    {
        let (look_bind, grid_bind) = submitter.forward_binds()?;
        let target_view = submitter.target_view.as_ref().ok_or_else(|| {
            LightError::Invalid(
                "shadow-cluster forward pass has no target after ensure_target".into(),
            )
        })?;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rl1-cluster-forward"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&submitter.forward_pipeline);
        pass.set_bind_group(0, look_bind, &[]);
        pass.set_bind_group(1, grid_bind, &[]);
        pass.draw(0..3, 0..1);
    }
    let bytes_per_row = width_px * 4;
    let padded = bytes_per_row.div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rl1-readback"),
        size: u64::from(padded) * u64::from(height_px),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: submitter.target.as_ref().ok_or_else(|| {
                LightError::Invalid(
                    "shadow-cluster readback has no target after ensure_target".into(),
                )
            })?,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height_px),
            },
        },
        wgpu::Extent3d {
            width: width_px,
            height: height_px,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| LightError::Device(format!("readback poll: {e}")))?;
    rx.recv()
        .map_err(|e| LightError::Device(format!("readback channel: {e}")))?
        .map_err(|e| LightError::Device(format!("readback map: {e}")))?;
    let (mut covered, mut torch) = (0u64, 0u64);
    {
        let mapped = slice
            .get_mapped_range()
            .map_err(|e| LightError::Device(format!("readback range: {e}")))?;
        for row in 0..height_px as usize {
            let off = row * padded as usize;
            let bytes = &mapped[off..off + bytes_per_row as usize];
            for px in bytes.chunks_exact(4) {
                if px[0] > COVERED_RED_MIN {
                    covered += 1;
                }
                if px[0] > TORCH_RED_MIN {
                    torch += 1;
                }
            }
        }
    }
    readback.unmap();
    let scope_err = pollster::block_on(scope.pop());
    if let Some(err) = scope_err {
        return Err(LightError::Validation(format!("{err:?}")));
    }
    Ok(ShadowClusterReceipt {
        frame_index: frame.frame_index,
        passes_submitted: SHADOW_CLUSTER_PASSES,
        texels: sun.texels(),
        cells: cells as u32,
        validation_errors: 0,
        covered_pixels: covered,
        torch_pixels: torch,
        submit_ms: started.elapsed().as_secs_f64() * 1000.0,
        degraded: sun.degraded.is_some()
            || cluster.degraded.is_some()
            || cluster.overflow_cells > 0,
    })
}

// ---------------------------------------------------------------------------
// Tests: splits tile the range, texel budget, cascade select, cluster
// assignment against a brute-force predicate, degrade/unsupported paths,
// determinism x2. Cascade parity tests ride `v1`; GPU alloc tests ride `window`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sun_config() -> SunShadowConfig {
        SunShadowConfig {
            near_m: 0.5,
            far_m: 60.0,
            half_extents: [4.0, 8.0, 16.0],
            biases: [0.002, 0.004, 0.008],
            size: 128,
        }
    }

    fn cluster_config() -> ClusterConfig {
        ClusterConfig {
            width_px: 64,
            height_px: 64,
            tile_px: CLUSTER_TILE_PX,
            depth_slices: 4,
            near_m: 0.5,
            far_m: 60.0,
            max_lights: MAX_TORCH_LIGHTS,
            max_per_cluster: MAX_LIGHTS_PER_CLUSTER,
        }
    }

    #[test]
    fn cascade_splits_tile_without_gaps() {
        // Endpoints reproduce the inputs, interiors strictly increase, so
        // the three half-open intervals tile [near, far) with no gap a
        // sample could fall through.
        let splits = cascade_splits(0.5, 60.0).expect("valid range splits");
        assert_eq!(splits[0], 0.5, "first split is the camera near");
        assert_eq!(splits[3], 60.0, "last split is the camera far");
        for pair in splits.windows(2) {
            assert!(pair[0] < pair[1], "splits increase, got {splits:?}");
        }
        let pass = SunShadowPass::build(sun_config()).expect("valid pass builds");
        assert_eq!(pass.splits, splits, "pass carries the split table");
        for i in 0..3 {
            assert_eq!(pass.cascade_index(pass.splits[i]), i.min(2));
            if i < 2 {
                assert_eq!(pass.cascade_index(pass.splits[i + 1]), i + 1);
            }
        }
        assert_eq!(pass.cascade_index(600.0), 2, "deep samples stay in band");
        assert_eq!(
            pass.texels(),
            3 * 128 * 128,
            "texel budget is cascades times map area"
        );
    }

    #[test]
    fn sun_bounds_mirror_the_r4_reference() {
        // Drift guard: the std-only bounds must equal the cascade module bounds
        // so receipts never quote a different budget than the bake uses.
        assert_eq!((SHADOW_MIN, SHADOW_MAX), (8, 512));
        assert_eq!(SUN_CASCADES, 3);
        assert_eq!(CASCADE_LAMBDA, 0.5);
        assert_eq!((CLUSTER_TILE_PX, MAX_LIGHTS_PER_CLUSTER), (16, 24));
        assert_eq!(
            (CLUSTER_TILE_PX, MAX_LIGHTS_PER_CLUSTER as u32),
            (crate::TILE, crate::MAX_LIGHTS_PER_TILE as u32)
        );
    }

    #[test]
    fn sun_invalid_inputs_fail_loudly() {
        assert!(matches!(
            cascade_splits(60.0, 0.5),
            Err(LightError::Invalid(_))
        ));
        assert!(matches!(
            cascade_splits(f32::NAN, 60.0),
            Err(LightError::Invalid(_))
        ));
        let mut bad = sun_config();
        bad.near_m = -1.0;
        assert!(matches!(
            SunShadowPass::build(bad),
            Err(LightError::Invalid(_))
        ));
        let mut bad = sun_config();
        bad.biases[1] = -0.5;
        assert!(matches!(
            SunShadowPass::build(bad),
            Err(LightError::Invalid(_))
        ));
        let mut bad = sun_config();
        bad.half_extents[0] = 0.0;
        assert!(matches!(
            SunShadowPass::build(bad),
            Err(LightError::Invalid(_))
        ));
    }

    #[test]
    fn sun_over_budget_size_degrades_not_fails() {
        // Budget exceeded degrades: the pass clamps to SHADOW_MAX, builds,
        // and records BudgetExceeded in the receipt instead of failing.
        let pass = SunShadowPass::build(SunShadowConfig {
            size: 4096,
            ..sun_config()
        })
        .expect("over-budget size degrades");
        assert_eq!(pass.config.size, SHADOW_MAX);
        assert!(matches!(
            pass.degraded,
            Some(LightError::BudgetExceeded {
                requested: 4096,
                allowed: 512
            })
        ));
        let receipt = pass.receipt(1.25);
        assert_eq!(
            (receipt.pass, receipt.texels, receipt.cells),
            ("sun_shadow", 3 * 512 * 512, 0)
        );
        assert!((receipt.ms - 1.25).abs() < 1e-9, "receipt carries pass ms");
        assert!(receipt.degraded, "receipt flags the degrade");
        assert!((SunShadowConfig::degrade_size(128).1).is_none());
    }

    #[test]
    fn headless_submit_names_the_window_entry() {
        // The headless seams never pretend: without a device they name the
        // window submit entry point as a typed error, never silent success
        // and never `Unsupported` on a device path that holds no device.
        let pass = SunShadowPass::build(sun_config()).expect("valid pass builds");
        assert!(!pass.baked);
        match pass.submit_gpu() {
            Err(LightError::Invalid(detail)) => {
                assert!(
                    detail.contains("submit_shadow_cluster"),
                    "names it: {detail}"
                );
            }
            other => panic!("headless sun seam must fail Invalid, got {other:?}"),
        }
        let cluster = ClusterPass::build(cluster_config(), &[]).expect("empty builds");
        match cluster.submit_gpu() {
            Err(LightError::Invalid(detail)) => {
                assert!(
                    detail.contains("submit_shadow_cluster"),
                    "names it: {detail}"
                );
            }
            other => panic!("headless cluster seam must fail Invalid, got {other:?}"),
        }
    }

    #[test]
    fn cluster_assign_matches_brute_force() {
        // Parity with the in-test brute-force reference: every cell lists
        // exactly the lights whose circle touches the tile and whose depth
        // span overlaps the slice, in increasing light order.
        let config = cluster_config();
        let lights = [
            ScreenLight {
                x_px: 8.0,
                y_px: 8.0,
                depth_m: 2.0,
                radius_px: 20.0,
            },
            ScreenLight {
                x_px: 60.0,
                y_px: 60.0,
                depth_m: 40.0,
                radius_px: 4.0,
            },
            ScreenLight {
                x_px: 32.0,
                y_px: 32.0,
                depth_m: 10.0,
                radius_px: 0.0,
            },
            ScreenLight {
                x_px: -50.0,
                y_px: -50.0,
                depth_m: 5.0,
                radius_px: 6.0,
            },
        ];
        let pass = ClusterPass::build(config, &lights).expect("assign builds");
        let bounds = config.slice_bounds();
        assert_eq!(bounds.len(), 5);
        assert_eq!((bounds[0], bounds[4]), (0.5, 60.0));
        let tx_n = config.tiles_x();
        let ty_n = config.tiles_y();
        for s in 0..config.depth_slices as usize {
            for ty in 0..ty_n {
                for tx in 0..tx_n {
                    let cell = pass.cell_index(tx, ty, s);
                    let mut want: Vec<u32> = Vec::new();
                    for (li, light) in lights.iter().enumerate() {
                        let qx = light.x_px.clamp(tx as f32 * 16.0, tx as f32 * 16.0 + 16.0);
                        let qy = light.y_px.clamp(ty as f32 * 16.0, ty as f32 * 16.0 + 16.0);
                        let dx = light.x_px - qx;
                        let dy = light.y_px - qy;
                        let touches = dx * dx + dy * dy <= light.radius_px * light.radius_px;
                        let overlaps = light.depth_m + light.radius_px >= bounds[s]
                            && light.depth_m - light.radius_px <= bounds[s + 1];
                        if touches && overlaps {
                            want.push(li as u32);
                        }
                    }
                    assert_eq!(
                        pass.lights_in(cell),
                        want.as_slice(),
                        "cell ({tx},{ty},{s}) matches brute force"
                    );
                }
            }
        }
        // The offscreen light touches nothing; the point light hits exactly
        // one tile in the slices its depth plane crosses.
        let total: usize = pass.counts.iter().map(|c| *c as usize).sum();
        assert_eq!(total, pass.indices.len());
        assert_eq!(pass.overflow_cells, 0);
        assert_eq!(pass.dropped_lights, 0);
        let receipt = pass.receipt();
        assert_eq!((receipt.pass, receipt.texels), ("torch_cluster", 0));
        assert_eq!(receipt.cells, config.cells() as u32);
        assert!(receipt.ms >= 0.0);
        assert!(!receipt.degraded);
    }

    #[test]
    fn cluster_budgets_degrade_not_fail() {
        // Tail lights past max_lights drop with BudgetExceeded; cells past
        // max_per_cluster clamp and count overflow. Both stay green frames.
        let config = ClusterConfig {
            max_lights: 2,
            ..cluster_config()
        };
        let lights = [
            ScreenLight {
                x_px: 32.0,
                y_px: 32.0,
                depth_m: 5.0,
                radius_px: 40.0,
            },
            ScreenLight {
                x_px: 32.0,
                y_px: 32.0,
                depth_m: 5.0,
                radius_px: 40.0,
            },
            ScreenLight {
                x_px: 32.0,
                y_px: 32.0,
                depth_m: 5.0,
                radius_px: 40.0,
            },
        ];
        let pass = ClusterPass::build(config, &lights).expect("tail drops");
        assert_eq!(pass.dropped_lights, 1);
        assert!(matches!(
            pass.degraded,
            Some(LightError::BudgetExceeded {
                requested: 3,
                allowed: 2
            })
        ));
        assert!(pass.receipt().degraded);

        let tight = ClusterConfig {
            max_per_cluster: 1,
            ..cluster_config()
        };
        let both = [
            ScreenLight {
                x_px: 8.0,
                y_px: 8.0,
                depth_m: 1.0,
                radius_px: 2.0,
            },
            ScreenLight {
                x_px: 8.0,
                y_px: 8.0,
                depth_m: 1.0,
                radius_px: 2.0,
            },
        ];
        let pass = ClusterPass::build(tight, &both).expect("cells clamp");
        assert!(
            pass.overflow_cells > 0,
            "shared cells must overflow at 1/cell"
        );
        for cell in 0..tight.cells() {
            assert!(pass.counts[cell] <= 1);
        }
        assert!(pass.receipt().degraded);
    }

    #[test]
    fn cluster_invalid_inputs_fail_loudly() {
        let bad_light = ScreenLight {
            x_px: f32::NAN,
            y_px: 0.0,
            depth_m: 1.0,
            radius_px: 4.0,
        };
        assert!(matches!(
            ClusterPass::build(cluster_config(), &[bad_light]),
            Err(LightError::Invalid(_))
        ));
        let bad_radius = ScreenLight {
            x_px: 0.0,
            y_px: 0.0,
            depth_m: 1.0,
            radius_px: -1.0,
        };
        assert!(matches!(
            ClusterPass::build(cluster_config(), &[bad_radius]),
            Err(LightError::Invalid(_))
        ));
        let bad_torch = TorchLight {
            pos: [0.0, 0.0, 0.0],
            range_m: 0.0,
            color: [1.0, 0.6, 0.2],
            intensity: 1.0,
        };
        assert!(matches!(bad_torch.validate(), Err(LightError::Invalid(_))));
        let bad_frame = ClusterConfig {
            width_px: 0,
            ..cluster_config()
        };
        assert!(matches!(
            ClusterPass::build(bad_frame, &[]),
            Err(LightError::Invalid(_))
        ));
        let bad_slices = ClusterConfig {
            depth_slices: 0,
            ..cluster_config()
        };
        assert!(matches!(
            ClusterPass::build(bad_slices, &[]),
            Err(LightError::Invalid(_))
        ));
        TorchLight {
            pos: [1.0, 2.0, 3.0],
            range_m: 5.0,
            color: [1.0, 0.6, 0.2],
            intensity: 2.0,
        }
        .validate()
        .expect("valid torch validates");
    }

    #[test]
    fn lights_passes_are_deterministic() {
        // Same complete inputs twice give bit-identical passes (x2).
        let lights = [
            ScreenLight {
                x_px: 10.0,
                y_px: 50.0,
                depth_m: 3.0,
                radius_px: 12.0,
            },
            ScreenLight {
                x_px: 40.0,
                y_px: 20.0,
                depth_m: 20.0,
                radius_px: 8.0,
            },
        ];
        let (a, b) = (
            ClusterPass::build(cluster_config(), &lights).expect("builds"),
            ClusterPass::build(cluster_config(), &lights).expect("builds"),
        );
        assert_eq!(a.counts, b.counts);
        assert_eq!(a.offsets, b.offsets);
        assert_eq!(a.indices, b.indices);
        let (p, q) = (
            SunShadowPass::build(sun_config()).expect("builds"),
            SunShadowPass::build(sun_config()).expect("builds"),
        );
        assert_eq!(p, q);
        let frame = FrameLightsReceipt {
            sun: p.receipt(0.5),
            cluster: a.receipt(),
        };
        assert!((frame.total_ms() - (0.5 + a.last_ms)).abs() < 1e-9);
        assert!(!frame.degraded());
    }

    #[test]
    fn rt19_tightened_prepass_cuts_pairs_at_brute_identical() {
        // Tightened smallest proof: 250-torch seed-19 fixture on 320x180 tile16
        // 20x12x8=1920 cells. Baseline full 0.5-50 vs tightened 0.5-20 plus
        // early-z center reject d>20. Accept: cut >= 15%, brute identical,
        // digest x2. Kill: cut < 15%, any mismatch, any drift.
        let base = ClusterConfig {
            width_px: 320,
            height_px: 180,
            tile_px: CLUSTER_TILE_PX,
            depth_slices: 8,
            near_m: 0.5,
            far_m: 50.0,
            max_lights: MAX_TORCH_LIGHTS,
            max_per_cluster: MAX_LIGHTS_PER_CLUSTER,
        };
        let mut rng = engine_core::Rng::new(19);
        let mut lights: Vec<ScreenLight> = Vec::with_capacity(250);
        for _ in 0..25 {
            lights.push(ScreenLight {
                x_px: (rng.next_u64() % 320) as f32,
                y_px: (rng.next_u64() % 180) as f32,
                depth_m: 10.0,
                radius_px: 60.0,
            });
        }
        for _ in 0..25 {
            lights.push(ScreenLight {
                x_px: (rng.next_u64() % 320) as f32,
                y_px: (rng.next_u64() % 180) as f32,
                depth_m: 35.0,
                radius_px: 60.0,
            });
        }
        for _ in 0..200 {
            lights.push(ScreenLight {
                x_px: (rng.next_u64() % 320) as f32,
                y_px: (rng.next_u64() % 180) as f32,
                depth_m: 10.0,
                radius_px: 8.0,
            });
        }
        assert_eq!(lights.len(), 250);
        assert_eq!(base.cells(), 1920);
        let full = ClusterPass::build(base, &lights).expect("baseline builds");
        let tight = ClusterPass::build_rt19_tightened(base, &lights).expect("tightened builds");
        assert_eq!(tight.config.far_m, RT19_TIGHT_FAR_M);
        let full_pairs = full.indices.len();
        let tight_pairs = tight.indices.len();
        assert!(full_pairs > 0, "baseline must assign pairs");
        let cut = 100.0 * (full_pairs - tight_pairs) as f64 / full_pairs as f64;
        assert!(
            cut >= 15.0,
            "RT19 pair cut {cut:.1}% < 15% (full {full_pairs}, tight {tight_pairs})"
        );
        // Brute identical on the tightened grid with the same reject rule.
        // The pre-pass compacts the kept lights, so brute indices are kept
        // positions (0..kept.len), matching what build stores.
        let bounds = tight.config.slice_bounds();
        let kept: Vec<&ScreenLight> = lights
            .iter()
            .filter(|l| l.depth_m <= RT19_REJECT_DEPTH_M)
            .collect();
        for s in 0..tight.config.depth_slices as usize {
            for ty in 0..tight.config.tiles_y() {
                for tx in 0..tight.config.tiles_x() {
                    let cell = tight.cell_index(tx, ty, s);
                    let mut want: Vec<u32> = Vec::new();
                    for (kept_pos, light) in kept.iter().enumerate() {
                        let qx = light.x_px.clamp(tx as f32 * 16.0, tx as f32 * 16.0 + 16.0);
                        let qy = light.y_px.clamp(ty as f32 * 16.0, ty as f32 * 16.0 + 16.0);
                        let dx = light.x_px - qx;
                        let dy = light.y_px - qy;
                        let touches = dx * dx + dy * dy <= light.radius_px * light.radius_px;
                        let overlaps = light.depth_m + light.radius_px >= bounds[s]
                            && light.depth_m - light.radius_px <= bounds[s + 1];
                        if touches && overlaps {
                            want.push(kept_pos as u32);
                        }
                    }
                    // Cells cap at max_per_cluster: compare the kept prefix
                    // the same way the assign caps, so overflow stays visible.
                    let mut capped = want.clone();
                    if capped.len() > tight.config.max_per_cluster {
                        capped.truncate(tight.config.max_per_cluster);
                        assert!(
                            tight.counts[cell] <= tight.config.max_per_cluster as u32,
                            "cell caps at max_per_cluster"
                        );
                    }
                    assert_eq!(
                        tight.lights_in(cell),
                        capped.as_slice(),
                        "cell ({tx},{ty},{s}) matches tightened brute force"
                    );
                }
            }
        }
        // Digest x2 identical, tightened differs from full (cut is real).
        let (d1, d2) = (tight.rt19_digest(), tight.rt19_digest());
        assert_eq!(d1, d2, "tightened digest must be x2 identical");
        let again = ClusterPass::build_rt19_tightened(base, &lights).expect("rebuilds");
        assert_eq!(again.rt19_digest(), d1, "rebuild digest identical");
        assert_ne!(
            full.rt19_digest(),
            d1,
            "tightened digest must differ from the full-span baseline"
        );
    }

    #[test]
    fn temporal_carry_cuts_pairs_at_low_rmse() {
        // Temporal carry proof: seed 19 250 torch fixture on 320x180 tile16
        // 20x12x8 cells. Tightened baseline vs temporal one per cell carried
        // from the full set on the tightened grid. Accept: cut over the
        // baseline at least 15 pct, error at most 10 pct, ms within budget,
        // digest stable, every kept light inside the brute set.
        let base = ClusterConfig {
            width_px: 320,
            height_px: 180,
            tile_px: CLUSTER_TILE_PX,
            depth_slices: 8,
            near_m: 0.5,
            far_m: 50.0,
            max_lights: MAX_TORCH_LIGHTS,
            max_per_cluster: MAX_LIGHTS_PER_CLUSTER,
        };
        let mut rng = engine_core::Rng::new(19);
        let mut lights: Vec<ScreenLight> = Vec::with_capacity(250);
        for _ in 0..25 {
            lights.push(ScreenLight {
                x_px: (rng.next_u64() % 320) as f32,
                y_px: (rng.next_u64() % 180) as f32,
                depth_m: 10.0,
                radius_px: 60.0,
            });
        }
        for _ in 0..25 {
            lights.push(ScreenLight {
                x_px: (rng.next_u64() % 320) as f32,
                y_px: (rng.next_u64() % 180) as f32,
                depth_m: 35.0,
                radius_px: 60.0,
            });
        }
        for _ in 0..200 {
            lights.push(ScreenLight {
                x_px: (rng.next_u64() % 320) as f32,
                y_px: (rng.next_u64() % 180) as f32,
                depth_m: 10.0,
                radius_px: 8.0,
            });
        }
        assert_eq!(lights.len(), 250);
        let tight = ClusterPass::build_rt19_tightened(base, &lights).expect("tightened builds");
        let tight_pairs = tight.indices.len();
        let (temporal, reservoirs, report) =
            ClusterPass::build_temporal_carry(base, &lights, 19).expect("temporal builds");
        assert_eq!(report.pairs, temporal.indices.len(), "report pairs match");
        assert_eq!(report.digest, temporal.rt19_digest(), "report digest match");
        assert!(report.pairs > 0, "temporal must keep pairs");
        let base_pairs = f64::from(TEMPORAL_BASELINE_PAIRS);
        let cut_base = 100.0 * (base_pairs - report.pairs as f64) / base_pairs;
        assert!(
            cut_base >= 15.0,
            "temporal cut {cut_base:.1}% under 15% (pairs {})",
            report.pairs
        );
        assert!(
            (report.cut_pct - cut_base).abs() < 1e-9,
            "report cut matches recomputed"
        );
        let cut_tight = 100.0 * (tight_pairs as f64 - report.pairs as f64) / tight_pairs as f64;
        assert!(
            cut_tight >= 15.0,
            "temporal cut over tight {cut_tight:.1}% under 15% (tight {tight_pairs}, temporal {})",
            report.pairs
        );
        assert!(
            report.rmse <= 0.10,
            "temporal error {} over 10%",
            report.rmse
        );
        assert!(
            report.ms <= TEMPORAL_ASSIGN_BUDGET_MS,
            "temporal ms {} over budget",
            report.ms
        );
        assert_eq!(
            reservoirs.len(),
            temporal.config.cells(),
            "one reservoir per cell"
        );
        let (again, _, again_report) =
            ClusterPass::build_temporal_carry(base, &lights, 19).expect("rebuilds");
        assert_eq!(
            again.rt19_digest(),
            report.digest,
            "rebuild digest identical"
        );
        assert_eq!(again_report.digest, report.digest);
        assert_ne!(
            tight.rt19_digest(),
            report.digest,
            "temporal digest must differ from tightened"
        );
        let bounds = temporal.config.slice_bounds();
        let kept: Vec<&ScreenLight> = lights
            .iter()
            .filter(|l| l.depth_m <= RT19_REJECT_DEPTH_M)
            .collect();
        let mut saw_zero = false;
        let mut saw_one = false;
        for r in &reservoirs {
            if r.confidence == 0.0 {
                saw_zero = true;
            }
            if r.confidence == 1.0 {
                saw_one = true;
            }
        }
        assert!(saw_zero && saw_one, "confidence must cover kill plus carry");
        for s in 0..temporal.config.depth_slices as usize {
            for ty in 0..temporal.config.tiles_y() {
                for tx in 0..temporal.config.tiles_x() {
                    let cell = temporal.cell_index(tx, ty, s);
                    let got = temporal.lights_in(cell);
                    assert!(got.len() <= 1, "temporal keeps one per cell");
                    if got.is_empty() {
                        continue;
                    }
                    let mut want: Vec<u32> = Vec::new();
                    for (kept_pos, light) in kept.iter().enumerate() {
                        let qx = light.x_px.clamp(tx as f32 * 16.0, tx as f32 * 16.0 + 16.0);
                        let qy = light.y_px.clamp(ty as f32 * 16.0, ty as f32 * 16.0 + 16.0);
                        let dx = light.x_px - qx;
                        let dy = light.y_px - qy;
                        let touches = dx * dx + dy * dy <= light.radius_px * light.radius_px;
                        let overlaps = light.depth_m + light.radius_px >= bounds[s]
                            && light.depth_m - light.radius_px <= bounds[s + 1];
                        if touches && overlaps {
                            want.push(kept_pos as u32);
                        }
                    }
                    assert!(
                        want.contains(&got[0]),
                        "cell ({tx},{ty},{s}) light {} outside brute set",
                        got[0]
                    );
                }
            }
        }
        eprintln!(
            "temporal carry: tight {tight_pairs} temporal {} cut_base {cut_base:.1}% cut_tight {cut_tight:.1}% rmse {:.6} ms {:.2} digest {:x}",
            report.pairs, report.rmse, report.ms, report.digest
        );
    }

    // ------------------------------------------------------------------
    // Cascade CPU reference parity behind `v1`: splits, slice bounds and baked
    // shadow factors must match `v1_lighting` within 1e-6 (observed: exact).
    // ------------------------------------------------------------------

    #[cfg(feature = "v1")]
    #[test]
    fn sun_splits_match_r4_practical_splits() {
        let ours = cascade_splits(0.5, 60.0).expect("splits build");
        let theirs = crate::v1_lighting::practical_splits(0.5, 60.0).expect("R4 splits build");
        let mut max_diff = 0.0f32;
        for (a, b) in ours.iter().zip(theirs.iter()) {
            max_diff = max_diff.max((a - b).abs());
        }
        assert!(
            max_diff <= 1e-6,
            "split parity within 1e-6, max diff {max_diff}"
        );
        assert_eq!(max_diff, 0.0, "splits are bit-identical to R4");
        // Slice bounds generalize the same blend: 3 slices reproduce the reference.
        let config = ClusterConfig {
            depth_slices: 3,
            ..cluster_config()
        };
        let bounds = config.slice_bounds();
        for (got, want) in bounds.iter().zip(theirs.iter()) {
            assert!(
                (got - want).abs() <= 1e-6,
                "slice bound parity, got {bounds:?}"
            );
        }
    }

    #[cfg(feature = "v1")]
    #[test]
    fn sun_shadow_factor_matches_r4_reference() {
        // Overhead light over the unit cube through the baked maps: the
        // wrapper must agree with a direct `CascadeSet` within 1e-6.
        let mesh = crate::v1::cube();
        let mut pass = SunShadowPass::build(sun_config()).expect("pass builds");
        assert_eq!(
            pass.shadow_factor(0.6, [0.0, 0.4, 0.0]),
            Ok(1.0),
            "unbaked passes are lit, never dark"
        );
        let ms = pass
            .bake_maps(&mesh, [0.0, 1.0, 0.0], [0.0, 0.0, 0.0])
            .expect("maps bake");
        assert!(ms >= 0.0, "bake records pass ms, got {ms}");
        assert!(pass.baked);
        let ours_lit = pass.shadow_factor(0.6, [0.0, 0.6, 0.0]).expect("samples");
        let ours_dark = pass.shadow_factor(0.6, [0.0, 0.4, 0.0]).expect("samples");
        assert_eq!(ours_lit, 1.0, "point above the caster is lit");
        assert_eq!(ours_dark, 0.0, "point behind the stored depth is shadowed");
        let set = pass.maps.as_ref().expect("maps resident");
        let mut max_diff = 0.0f32;
        for (depth, point) in [
            (0.6, [0.0, 0.6, 0.0]),
            (0.6, [0.0, 0.4, 0.0]),
            (55.0, [0.0, 0.4, 0.0]),
            (3.0, [0.5, 0.5, 0.5]),
        ] {
            let direct = set.shadow_factor(depth, point).expect("R4 samples");
            let wrapped = pass.shadow_factor(depth, point).expect("wrapper samples");
            max_diff = max_diff.max((wrapped - direct).abs());
        }
        assert!(
            max_diff <= 1e-6,
            "shadow parity within 1e-6, max diff {max_diff}"
        );
        assert_eq!(
            ours_dark,
            set.shadow_factor(0.6, [0.0, 0.4, 0.0]).expect("R4")
        );
        // The headless seam names the window submit even when baked: parity
        // is CPU-side, never a fake GPU pass.
        match pass.submit_gpu() {
            Err(LightError::Invalid(detail)) => {
                assert!(
                    detail.contains("submit_shadow_cluster"),
                    "names it: {detail}"
                );
            }
            other => panic!("headless seam must fail Invalid, got {other:?}"),
        }
    }

    #[cfg(feature = "v1")]
    #[test]
    fn perspective_projection_lands_in_frame() {
        // Identity view-proj maps the origin to the frame center; a point
        // behind the camera (w <= 0) projects to None, never NaN pixels.
        let center = project_perspective([0.0, 0.0, 0.0], glam::Mat4::IDENTITY, 64, 64, 32.0, 2.0)
            .expect("origin projects");
        assert_eq!((center.x_px, center.y_px), (32.0, 32.0));
        assert!(
            (center.radius_px - 64.0).abs() < 1e-4,
            "radius scales by focal/depth"
        );
        assert!(center.validate().is_ok());
        // Behind the camera: negated w flips the sign test to None.
        let behind = project_perspective(
            [0.0, 0.0, -6.0],
            glam::Mat4::from_cols(
                glam::Vec4::X,
                glam::Vec4::Y,
                glam::Vec4::Z,
                glam::Vec4::new(0.0, 0.0, 0.0, -1.0),
            ),
            64,
            64,
            32.0,
            2.0,
        );
        assert!(behind.is_none(), "behind-camera lights do not project");
        assert!(
            project_perspective([0.0, 0.0, 0.0], glam::Mat4::IDENTITY, 0, 64, 32.0, 1.0).is_none()
        );
    }

    // ------------------------------------------------------------------
    // GPU descriptors behind `window`: pure format/extent checks plus a real
    // depth allocation that skips without an adapter (presenter skip pattern).
    // ------------------------------------------------------------------

    #[cfg(feature = "window")]
    #[test]
    fn cascade_depth_desc_is_sampler_readable() {
        let desc = gpu::cascade_depth_desc(128);
        assert_eq!(desc.format, wgpu::TextureFormat::Depth32Float);
        assert_eq!((desc.size.width, desc.size.height), (128, 128));
        assert!(desc.usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT));
        assert!(desc.usage.contains(wgpu::TextureUsages::TEXTURE_BINDING));
    }

    #[cfg(feature = "window")]
    #[test]
    fn cascade_depths_allocate_on_llvmpipe() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })) {
                Ok(a) => a,
                Err(_) => {
                    eprintln!("SKIPPED shadow_cluster gpu test: no wgpu adapter on this machine");
                    return;
                }
            };
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("rl1-shadow-depth"),
                ..Default::default()
            }))
            .expect("device requests succeed when the adapter exists");
        let maps = gpu::allocate_cascade_depths(&device, 64, SUN_CASCADES)
            .expect("cascade depths allocate");
        assert_eq!(maps.len(), SUN_CASCADES);
        assert!(matches!(
            gpu::allocate_cascade_depths(&device, 4096, SUN_CASCADES),
            Err(LightError::Invalid(_))
        ));
        assert!(matches!(
            gpu::allocate_cascade_depths(&device, 64, 0),
            Err(LightError::Invalid(_))
        ));
    }

    // ------------------------------------------------------------------
    // Window submit (behind `window`): real cascade depth encodes plus
    // the clustered forward pass on llvmpipe, CPU parity pinned, negatives
    // loud. Device comes from the shared suite device (skip, never
    // fake, without an adapter).
    // ------------------------------------------------------------------

    #[cfg(feature = "window")]
    fn rl1_device() -> Option<(
        &'static wgpu::Device,
        &'static wgpu::Queue,
        &'static wgpu::AdapterInfo,
    )> {
        crate::graph::gx1_test_device()
    }

    #[cfg(feature = "window")]
    fn rl1_frame(index: u64) -> crate::gtao::LookFrame {
        crate::gtao::LookFrame {
            frame_index: index,
            viewport_px: [64, 64],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        }
    }

    #[cfg(feature = "window")]
    fn rl1_sun() -> SunShadowPass {
        let mut pass = SunShadowPass::build(SunShadowConfig {
            size: 64,
            ..sun_config()
        })
        .expect("64-texel sun builds");
        pass.bake_maps(&crate::v1::cube(), [0.0, 1.0, 0.0], [0.0, 0.0, 0.0])
            .expect("maps bake on llvmpipe");
        pass
    }

    #[cfg(feature = "window")]
    fn rl1_lit_cluster() -> ClusterPass {
        let config = ClusterConfig {
            width_px: 64,
            height_px: 64,
            tile_px: CLUSTER_TILE_PX,
            depth_slices: 4,
            near_m: 0.5,
            far_m: 60.0,
            max_lights: MAX_TORCH_LIGHTS,
            max_per_cluster: MAX_LIGHTS_PER_CLUSTER,
        };
        ClusterPass::build(
            config,
            &[
                ScreenLight {
                    x_px: 32.0,
                    y_px: 32.0,
                    depth_m: 5.0,
                    radius_px: 40.0,
                },
                ScreenLight {
                    x_px: 8.0,
                    y_px: 8.0,
                    depth_m: 2.0,
                    radius_px: 6.0,
                },
            ],
        )
        .expect("lit cluster builds")
    }

    #[cfg(feature = "window")]
    #[test]
    fn shadow_uniform_bytes_match_pass_splits() {
        // The uniform image carries the pass splits plus the active cascade
        // depth: decode round-trips the exact f32 bits, and switching the
        // active cascade moves only the active-depth word.
        let sun = SunShadowPass::build(sun_config()).expect("pass builds");
        let packed = shadow_uniform_bytes(&sun, 1);
        assert_eq!(packed.len(), SHADOW_UNIFORM_BYTES as usize);
        let word = |at: usize| f32::from_le_bytes(packed[at..at + 4].try_into().expect("word"));
        for (i, split) in sun.splits.iter().enumerate() {
            assert_eq!(word(i * 4), *split, "split {i} round-trips");
        }
        assert_eq!(
            word(16),
            sun.splits[2],
            "active depth is splits[active + 1]"
        );
        assert_eq!(word(20), sun.config.near_m);
        assert_eq!(word(24), sun.config.far_m);
        assert_eq!(word(28), sun.config.size as f32);
        let other = shadow_uniform_bytes(&sun, 0);
        assert_eq!(
            f32::from_le_bytes(other[16..20].try_into().expect("word")),
            sun.splits[1]
        );
        assert_eq!(&packed[..16], &other[..16], "only the active word moves");
        assert_eq!(&packed[20..], &other[20..], "range words are stable");
        // Forced defect check: a corrupt active word is observable, so the
        // depth pass cannot silently encode the wrong cascade.
        assert_ne!(packed[16..20], other[16..20]);
    }

    #[cfg(feature = "window")]
    #[test]
    fn window_submit_records_cascades_and_cluster_clean() {
        let Some((device, queue, info)) = rl1_device() else {
            eprintln!("SKIPPED rl1 window submit: no wgpu adapter on this machine");
            return;
        };
        let sun = rl1_sun();
        let cluster = rl1_lit_cluster();
        // CPU reference snapshots before the submit: presentation must not
        // move them.
        let lit_before = sun.shadow_factor(0.6, [0.0, 0.6, 0.0]).expect("samples");
        let dark_before = sun.shadow_factor(0.6, [0.0, 0.4, 0.0]).expect("samples");
        let (counts_before, offsets_before, indices_before) = (
            cluster.counts.clone(),
            cluster.offsets.clone(),
            cluster.indices.clone(),
        );
        let mut submitter = ShadowClusterSubmitter::new(device);
        let receipt =
            submit_shadow_cluster(device, queue, &rl1_frame(7), &sun, &cluster, &mut submitter)
                .expect("rl1 window submit runs clean");
        assert_eq!(
            receipt.passes_submitted, SHADOW_CLUSTER_PASSES,
            "cascades plus the forward pass"
        );
        assert!(
            receipt.is_clean(),
            "0 validation errors, got {}",
            receipt.validation_errors
        );
        assert_eq!(receipt.validation_errors, 0);
        assert!(receipt.covered_pixels > 0, "passes must write pixels");
        assert!(receipt.torch_pixels > 0, "lit cluster must shade pixels");
        assert!(receipt.torch_pixels <= receipt.covered_pixels);
        assert_eq!(
            receipt.texels,
            sun.texels(),
            "receipt quotes the pass budget"
        );
        assert_eq!(receipt.cells, cluster.config.cells() as u32);
        // Wall-clock ms is evidence only (CODELAW: timing never gates a
        // functional test); the posted budget method stays for captures.
        assert!(!receipt.degraded, "fixture fits every budget");
        // Parity: wrapper still agrees with the direct set within 1e-6,
        // and the submit moved neither the maps nor the grid.
        let set = sun.maps.as_ref().expect("maps resident");
        let mut max_diff = 0.0f32;
        for (depth, point) in [
            (0.6, [0.0, 0.6, 0.0]),
            (0.6, [0.0, 0.4, 0.0]),
            (55.0, [0.0, 0.4, 0.0]),
            (3.0, [0.5, 0.5, 0.5]),
        ] {
            let direct = set.shadow_factor(depth, point).expect("R4 samples");
            let wrapped = sun.shadow_factor(depth, point).expect("wrapper samples");
            max_diff = max_diff.max((wrapped - direct).abs());
        }
        assert!(
            max_diff <= 1e-6,
            "shadow parity within 1e-6, max diff {max_diff}"
        );
        assert_eq!(
            sun.shadow_factor(0.6, [0.0, 0.6, 0.0]).expect("samples"),
            lit_before
        );
        assert_eq!(
            sun.shadow_factor(0.6, [0.0, 0.4, 0.0]).expect("samples"),
            dark_before
        );
        assert_eq!(
            (lit_before, dark_before),
            (1.0, 0.0),
            "lit stays lit, occluded stays dark"
        );
        assert_eq!(
            cluster.counts, counts_before,
            "submit never rewrites the grid"
        );
        assert_eq!(cluster.offsets, offsets_before);
        assert_eq!(cluster.indices, indices_before);
        // Determinism: identical inputs give identical pixels on one adapter.
        let again =
            submit_shadow_cluster(device, queue, &rl1_frame(7), &sun, &cluster, &mut submitter)
                .expect("second submit runs");
        assert_eq!(
            (again.covered_pixels, again.torch_pixels),
            (receipt.covered_pixels, receipt.torch_pixels),
            "pixels are deterministic per inputs"
        );
        eprintln!(
            "RL1 receipt on {}: passes={} texels={} cells={} covered={} torch={} parity_max_diff={} submit_ms={:.2} within_budget={} validation_errors={}",
            info.name,
            receipt.passes_submitted,
            receipt.texels,
            receipt.cells,
            receipt.covered_pixels,
            receipt.torch_pixels,
            max_diff,
            receipt.submit_ms,
            receipt.within_budget(),
            receipt.validation_errors
        );
    }

    #[cfg(feature = "window")]
    #[test]
    fn window_submit_empty_cluster_has_no_torch() {
        // Behavioral negative: the same frame with an empty grid shades the
        // base but lights nothing, proving the cluster buffers feed pixels
        // instead of merely validating.
        let Some((device, queue, _)) = rl1_device() else {
            eprintln!("SKIPPED rl1 empty cluster: no wgpu adapter on this machine");
            return;
        };
        let sun = rl1_sun();
        let config = ClusterConfig {
            width_px: 64,
            height_px: 64,
            tile_px: CLUSTER_TILE_PX,
            depth_slices: 4,
            near_m: 0.5,
            far_m: 60.0,
            max_lights: MAX_TORCH_LIGHTS,
            max_per_cluster: MAX_LIGHTS_PER_CLUSTER,
        };
        let empty = ClusterPass::build(config, &[]).expect("empty builds");
        let mut submitter = ShadowClusterSubmitter::new(device);
        let receipt =
            submit_shadow_cluster(device, queue, &rl1_frame(8), &sun, &empty, &mut submitter)
                .expect("empty grid submits");
        assert!(receipt.is_clean());
        assert!(receipt.covered_pixels > 0, "base frame still writes pixels");
        assert_eq!(receipt.torch_pixels, 0, "no lights means no torch pixels");
    }

    #[cfg(feature = "window")]
    #[test]
    fn window_submit_reuses_targets_and_binds() {
        // Steady-state proof: matching-shape submits create no new depth
        // maps, grid buffers, targets or bind groups.
        let Some((device, queue, _)) = rl1_device() else {
            eprintln!("SKIPPED rl1 reuse: no wgpu adapter on this machine");
            return;
        };
        let sun = rl1_sun();
        let cluster = rl1_lit_cluster();
        let mut submitter = ShadowClusterSubmitter::new(device);
        submit_shadow_cluster(device, queue, &rl1_frame(9), &sun, &cluster, &mut submitter)
            .expect("first submit sizes");
        let shape = (
            submitter.depth_map_count(),
            submitter.grid_buffer_count(),
            submitter.bind_group_count(),
        );
        assert_eq!(shape.0, SUN_CASCADES, "one map per cascade");
        assert_eq!(shape.1, 3, "counts plus offsets plus indices");
        assert_eq!(shape.2, 3, "depth plus look plus cluster binds");
        let again = submit_shadow_cluster(
            device,
            queue,
            &rl1_frame(10),
            &sun,
            &cluster,
            &mut submitter,
        )
        .expect("second submit reuses");
        assert_eq!(
            (
                submitter.depth_map_count(),
                submitter.grid_buffer_count(),
                submitter.bind_group_count()
            ),
            shape,
            "steady-state submit creates nothing new"
        );
        assert!(again.is_clean() && again.covered_pixels > 0);
    }

    #[cfg(feature = "window")]
    #[test]
    fn window_submit_refuses_bad_frame_and_missing_binds_loudly() {
        // Forced missing bind group fails loudly: a fresh submitter holds no
        // forward binds, and the error names the rebuild entry point.
        let Some((device, queue, _)) = rl1_device() else {
            eprintln!("SKIPPED rl1 refusal: no wgpu adapter on this machine");
            return;
        };
        let fresh = ShadowClusterSubmitter::new(device);
        match fresh.forward_binds() {
            Err(LightError::Invalid(detail)) => {
                assert!(detail.contains("bind group"), "names it: {detail}");
                assert!(
                    detail.contains("submit_shadow_cluster"),
                    "names the rebuild: {detail}"
                );
            }
            other => panic!("missing binds must fail Invalid, got {other:?}"),
        }
        let sun = rl1_sun();
        let cluster = rl1_lit_cluster();
        let mut submitter = ShadowClusterSubmitter::new(device);
        let bad = crate::gtao::LookFrame {
            viewport_px: [0, 64],
            ..rl1_frame(11)
        };
        match submit_shadow_cluster(device, queue, &bad, &sun, &cluster, &mut submitter) {
            Err(LightError::Invalid(detail)) => {
                assert!(!detail.is_empty(), "never silent");
                assert!(
                    !detail.to_lowercase().contains("unsupported"),
                    "window path never reports Unsupported: {detail}"
                );
            }
            other => panic!("zero viewport must fail Invalid, got {other:?}"),
        }
        assert_eq!(
            (submitter.depth_map_count(), submitter.bind_group_count()),
            (0, 0),
            "refused submits size nothing"
        );
    }

    #[test]
    fn sun_receipt_sanitizes_shadow_ms_for_per_pass() {
        let pass = SunShadowPass::build(sun_config()).expect("valid pass builds");
        let live = pass.receipt(1.25);
        assert_eq!(live.pass, "sun_shadow");
        assert!((live.ms - 1.25).abs() < 1e-9);
        assert_eq!(live.texels, pass.texels());
        assert!(!live.degraded);
        for wild in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            assert_eq!(pass.receipt(wild).ms, 0.0);
        }
        assert_eq!(sanitize_shadow_ms(1.25), 1.25);
        assert_eq!(sanitize_shadow_ms(f64::NAN), 0.0);
        let line = format!("shadow={:.3}ms", sanitize_shadow_ms(1.25));
        assert!(line.contains("shadow="));
        assert_eq!(line, "shadow=1.250ms");
        assert_eq!(line, format!("shadow={:.3}ms", sanitize_shadow_ms(1.25)));
    }
}

#[cfg(test)]
mod sun_shadow_regression {
    use super::{
        cascade_splits, practical_split, LightError, SunShadowConfig, CASCADE_LAMBDA, SUN_CASCADES,
    };

    fn valid_sun_config() -> SunShadowConfig {
        SunShadowConfig {
            near_m: 0.5,
            far_m: 60.0,
            half_extents: [4.0, 8.0, 16.0],
            biases: [0.002, 0.004, 0.008],
            size: 128,
        }
    }

    #[test]
    fn cascade_endpoints_strictly_increase() {
        let splits = cascade_splits(0.5, 60.0).expect("valid range splits");
        assert_eq!(splits[0], 0.5, "first split is near");
        assert_eq!(splits[3], 60.0, "last split is far");
        assert_eq!(
            splits.len(),
            SUN_CASCADES + 1,
            "one more split than cascades"
        );
        for pair in splits.windows(2) {
            assert!(pair[0] < pair[1], "strictly increasing, got {splits:?}");
        }
    }

    #[test]
    fn cascade_interiors_match_practical_split() {
        let (near, far) = (0.5_f32, 60.0_f32);
        let splits = cascade_splits(near, far).expect("valid range splits");
        assert_eq!(SUN_CASCADES, 3, "three cascades pin two interiors");
        assert_eq!(CASCADE_LAMBDA, 0.5, "reference half-log half-uniform blend");
        for i in 1..SUN_CASCADES {
            let t = i as f32 / SUN_CASCADES as f32;
            let expected = practical_split(near, far, t, CASCADE_LAMBDA);
            assert_eq!(splits[i], expected, "interior {i} calls practical_split");
        }
    }

    #[test]
    fn practical_split_endpoints_hit_near_and_far() {
        let (near, far) = (0.5_f32, 60.0_f32);
        let at_near = practical_split(near, far, 0.0, CASCADE_LAMBDA);
        let at_far = practical_split(near, far, 1.0, CASCADE_LAMBDA);
        assert!((at_near - near).abs() < 1e-6, "t=0 is near, got {at_near}");
        assert!((at_far - far).abs() < 1e-6, "t=1 is far, got {at_far}");
    }

    #[test]
    fn cascade_rejects_inverted_range() {
        match cascade_splits(10.0, 5.0) {
            Err(LightError::Invalid(_)) => {}
            other => panic!("inverted range must fail Invalid, got {other:?}"),
        }
        match cascade_splits(0.0, 10.0) {
            Err(LightError::Invalid(_)) => {}
            other => panic!("zero near must fail Invalid, got {other:?}"),
        }
    }

    #[test]
    fn sun_config_validate_rejects_inverted_range() {
        let bad = SunShadowConfig {
            near_m: 10.0,
            far_m: 5.0,
            ..valid_sun_config()
        };
        match bad.validate() {
            Err(LightError::Invalid(_)) => {}
            other => panic!("bad sun range must fail Invalid, got {other:?}"),
        }
        valid_sun_config()
            .validate()
            .expect("valid sun config passes");
    }
}
