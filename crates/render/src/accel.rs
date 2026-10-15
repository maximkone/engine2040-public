//! accel: BLAS per proxy mesh plus per-frame TLAS refit.
//!
//! Owner: render lane. Consumer: match frame (arena plus 8 skinned
//! fighters refit per frame); later ray passes read the TLAS next. Units: meters
//! for positions and AABBs, counts for tris/verts/instances, ms in receipts.
//! Determinism: index-order loops, sorted instance order, FNV-1a64 digests;
//! same proxy bytes build identical input bytes twice. Errors are typed
//! [`AccelError`]; Base tier falls back loudly, never silently.
//! Budget: TLAS plus skinned refit inside [`ACCEL_REFIT_BUDGET_MS`] (1.5 ms)
//! on the reference GPU; steady-state refits reuse every allocation.
//!
//! Approach (patterns only, engine-owned math): simplified RT proxies beside
//! full-resolution raster in the Bevy Solari shape, wgpu ray-query plus
//! accel flag shapes for the tier gate; no donor code vendored.

use core::fmt;
use std::time::Instant;

use engine_core::Fnv64;

use crate::tier::{AdapterCaps, Tier, TierError};

// ---------------------------------------------------------------------------
// Bounds: every external size is validated before allocation.
// ---------------------------------------------------------------------------

/// Posted per-frame refit budget on the reference GPU in ms: TLAS rebuild
/// plus skinned BLAS refits for the match frame (enforced at arena size).
pub const ACCEL_REFIT_BUDGET_MS: f64 = 1.5;
/// Most BLAS entries one scene build holds (arena plus fighters plus margin).
pub const ACCEL_MAX_BLAS: usize = 64;
/// Most instances one TLAS holds (arena plus 8 fighters, with margin).
pub const ACCEL_MAX_INSTANCES: usize = 64;
/// Most triangles one proxy mesh holds. Proxies are simplified LODs;
/// full detail stays raster, so oversized proxies are refused with the count
/// instead of being silently decimated (deviation is checked at build).
pub const ACCEL_MAX_PROXY_TRIS: u32 = 65_536;
/// Most vertices one proxy mesh holds; bounds the refit payload.
pub const ACCEL_MAX_PROXY_VERTS: usize = 65_536;
/// AABB containment slack in meters covering f32 rounding on the boundary.
pub const ACCEL_AABB_EPS_M: f32 = 1e-4;
/// WIN1 match fighters; the arena shell is instance zero beside these.
pub const WIN1_FIGHTERS: usize = 8;
/// WIN1 TLAS instances per frame: arena plus one per fighter.
pub const WIN1_INSTANCES: usize = WIN1_FIGHTERS + 1;
/// BLAS id of the arena shell in the WIN1 layout.
pub const ARENA_BLAS_ID: u32 = 0;
/// Graph pass name for the proxy BLAS build.
pub const ACCEL_BUILD_PASS: &str = "accel-build";
/// Graph pass name for the per-frame TLAS refit.
pub const ACCEL_REFIT_PASS: &str = "accel-refit";
/// Graph resource name for the built BLAS set.
pub const ACCEL_BLAS_RESOURCE: &str = "blas";
/// Graph resource name for the refit TLAS the RT passes read.
pub const ACCEL_TLAS_RESOURCE: &str = "tlas";
/// Device scratch estimate per proxy triangle in bytes (per-tri AABB plus
/// index words); sizes the RA2 transient, never the merge gate.
pub const ACCEL_BLAS_SCRATCH_PER_TRI_BYTES: u64 = 32;
/// Device bytes per TLAS instance descriptor (row-major 3x4 plus fields).
pub const ACCEL_TLAS_INSTANCE_BYTES: u64 = 64;

// ---------------------------------------------------------------------------
// Errors: typed; Base tier falls back loudly, the headless twin names the
// device bind it cannot run without, the window build never takes that path.
// ---------------------------------------------------------------------------

/// Acceleration-structure failures.
#[derive(Clone, PartialEq, Debug)]
pub enum AccelError {
    /// Caller input is unusable; reason carried.
    Invalid(String),
    /// Proxy exceeds the BLAS budget; counts carried.
    OverBudget {
        /// Triangles or vertices in the refused proxy.
        count: usize,
        /// Budget it exceeded.
        max: usize,
    },
    /// Proxy carries no cookable surface; bounded reason carried.
    Degenerate(String),
    /// RT path refused on this tier: names the request, the honest fallback
    /// and the missing capability. Base is never a silent downgrade.
    TierFallback {
        /// Tier the caller asked the RT path for.
        requested: Tier,
        /// Tier that honestly runs here instead.
        fallback: Tier,
        /// Missing capability or refused label.
        reason: String,
        /// Adapter name behind the refusal.
        adapter: String,
    },
    /// Refit got a new topology (vertex count changed); rebuild loudly.
    NeedsRebuild {
        /// Vertices the BLAS was built with.
        built: usize,
        /// Vertices the refit carried.
        got: usize,
    },
    /// No device behind this call; what to build first, carried.
    NoDevice(String),
    /// A device, queue or map call failed; message carried.
    Device(String),
}

impl fmt::Display for AccelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(detail) => write!(f, "accel input: {detail}"),
            Self::OverBudget { count, max } => {
                write!(f, "accel over budget: {count} over {max}")
            }
            Self::Degenerate(detail) => write!(f, "accel degenerate proxy: {detail}"),
            Self::TierFallback {
                requested,
                fallback,
                reason,
                adapter,
            } => write!(
                f,
                "accel RT path refused tier {requested} on adapter \"{adapter}\": \
                 {reason} (honest fallback: {fallback})"
            ),
            Self::NeedsRebuild { built, got } => write!(
                f,
                "accel refit needs rebuild: topology changed from {built} to {got} verts"
            ),
            Self::NoDevice(detail) => write!(f, "accel bind needs a device: {detail}"),
            Self::Device(detail) => write!(f, "accel device: {detail}"),
        }
    }
}

impl std::error::Error for AccelError {}

/// Map a tier refusal into the loud accel fallback, keeping the adapter name
/// and the missing capability on the error instead of downgrading silently.
fn map_tier_error(error: TierError) -> AccelError {
    match error {
        TierError::Unsupported {
            requested,
            fallback,
            reason,
            adapter,
        } => AccelError::TierFallback {
            requested,
            fallback,
            reason: reason.to_owned(),
            adapter,
        },
        TierError::UnknownTier(name) => AccelError::Invalid(format!(
            "unknown lighting tier: {name} (want base, rt or ultra)"
        )),
    }
}

// ---------------------------------------------------------------------------
// Tier gate: probing picks the tier; Base stays off the ray path.
// ---------------------------------------------------------------------------

/// Gate the ray submit path on the probed tier: Rt and Ultra pass, Base falls
/// back loudly to the cascade plus probe path instead of
/// running ray queries nowhere. Pure check, allocates nothing.
pub fn require_rt(tier: Tier) -> Result<(), AccelError> {
    match tier {
        Tier::Rt | Tier::Ultra => Ok(()),
        Tier::Base => Err(AccelError::TierFallback {
            requested: Tier::Base,
            fallback: Tier::Base,
            reason: "base tier runs RL1 cascades plus RL5/RT8 probes; \
                     the RT path needs ray queries plus accel"
                .to_owned(),
            adapter: String::new(),
        }),
    }
}

/// Resolve the startup tier for a match: `None` probes the adapter, `Some`
/// forces it and refuses loudly when the adapter lacks the features. Base
/// resolves fine (the CPU parity oracle runs everywhere); the device submit
/// still calls [`require_rt`] so Base never runs RT passes silently.
pub fn resolve_for_match(caps: &AdapterCaps, forced: Option<Tier>) -> Result<Tier, AccelError> {
    crate::tier::resolve(caps, forced)
        .map(|selection| selection.tier)
        .map_err(map_tier_error)
}

// ---------------------------------------------------------------------------
// Proxy input: raw LOD views. The cook owns `CookedMesh`; this module
// takes position/index slices so render needs no new dependency edge (the
// assets seam feeds `cooked.positions` plus `cooked.indices` straight
// in; the orchestrator owns declaring that composition edge).
// ---------------------------------------------------------------------------

/// One simplified proxy mesh: positions in meters plus triangle indices.
/// Full detail stays raster; this is the ray-query shape only.
#[derive(Clone, Copy, Debug)]
pub struct ProxyMesh<'a> {
    /// Positions in meters, one per vertex.
    pub positions: &'a [[f32; 3]],
    /// Triangle indices into `positions`, length a multiple of 3.
    pub indices: &'a [u32],
}

impl<'a> ProxyMesh<'a> {
    /// Validate before any digest or allocation: bounds, finiteness, index
    /// range and triangle count. Degenerate (all zero-area) proxies are
    /// refused here; partly degenerate ones build with a count (see
    /// [`Blas::degenerate_tris`]).
    pub fn validate(&self) -> Result<(), AccelError> {
        if self.positions.is_empty() {
            return Err(AccelError::Invalid("accel proxy holds no vertices".into()));
        }
        if self.positions.len() > ACCEL_MAX_PROXY_VERTS {
            return Err(AccelError::OverBudget {
                count: self.positions.len(),
                max: ACCEL_MAX_PROXY_VERTS,
            });
        }
        if self.indices.is_empty() || !self.indices.len().is_multiple_of(3) {
            return Err(AccelError::Invalid(format!(
                "accel proxy index length {} is empty or not a triangle multiple",
                self.indices.len()
            )));
        }
        let tris = self.indices.len() / 3;
        if tris > ACCEL_MAX_PROXY_TRIS as usize {
            return Err(AccelError::OverBudget {
                count: tris,
                max: ACCEL_MAX_PROXY_TRIS as usize,
            });
        }
        for (i, p) in self.positions.iter().enumerate() {
            if !p.iter().all(|v| v.is_finite()) {
                return Err(AccelError::Invalid(format!(
                    "accel proxy positions[{i}] is not finite"
                )));
            }
        }
        let bound = self.positions.len() as u32;
        for index in self.indices {
            if *index >= bound {
                return Err(AccelError::Invalid(format!(
                    "accel proxy index {index} out of range for {} verts",
                    self.positions.len()
                )));
            }
        }
        if largest_tri_area_m2(self.positions, self.indices) <= 1e-12 {
            return Err(AccelError::Degenerate(format!(
                "all {tris} proxy triangles zero-area"
            )));
        }
        Ok(())
    }

    /// Triangle count.
    pub fn tri_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// Largest triangle area in meters squared, index order. Single area
/// evaluation per triangle in one pass (was three per triangle).
fn largest_tri_area_m2(positions: &[[f32; 3]], indices: &[u32]) -> f32 {
    let mut worst = 0.0f32;
    for tri in indices.chunks_exact(3) {
        let (Some(p0), Some(p1), Some(p2)) = (
            positions.get(tri[0] as usize),
            positions.get(tri[1] as usize),
            positions.get(tri[2] as usize),
        ) else {
            continue;
        };
        let area = tri_area_m2(p0, p1, p2);
        if area.is_finite() && area > worst {
            worst = area;
        }
    }
    worst
}

/// One triangle area in meters squared.
fn tri_area_m2(p0: &[f32; 3], p1: &[f32; 3], p2: &[f32; 3]) -> f32 {
    let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
    let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
    let cross = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    0.5 * (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt()
}

// ---------------------------------------------------------------------------
// BLAS: CPU mirror of one bottom-level structure built from a proxy.
// ---------------------------------------------------------------------------

/// Single-pass post-validate build scan: one vertex loop (AABB) plus one
/// triangle loop (degenerate count, single area evaluation each) plus the
/// streamed digest fed inside those same two loops. The digest feeds id,
/// counts, every position bit and every index word in the same order
/// [`build_input_bytes_raw`] emits, so it is bit-identical with no
/// temporary image. Callers run [`ProxyMesh::validate`] first, so bounds
/// and finiteness hold here and error precedence never changes.
fn build_scan(
    id: u32,
    positions: &[[f32; 3]],
    indices: &[u32],
) -> ([f32; 3], [f32; 3], usize, u64) {
    let mut hasher = Fnv64::new();
    hasher.write(&id.to_le_bytes());
    hasher.write(&(positions.len() as u64).to_le_bytes());
    hasher.write(&(indices.len() as u64).to_le_bytes());
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in positions {
        for i in 0..3 {
            if p[i] < min[i] {
                min[i] = p[i];
            }
            if p[i] > max[i] {
                max[i] = p[i];
            }
            hasher.write(&p[i].to_le_bytes());
        }
    }
    let mut degenerate = 0usize;
    for tri in indices.chunks_exact(3) {
        // Validated in range, so direct indexing is exact here.
        let area = tri_area_m2(
            &positions[tri[0] as usize],
            &positions[tri[1] as usize],
            &positions[tri[2] as usize],
        );
        if area <= 1e-12 {
            degenerate += 1;
        }
        for corner in tri {
            hasher.write(&corner.to_le_bytes());
        }
    }
    (min, max, degenerate, hasher.finish())
}

/// One BLAS: the proxy vertex/index mirror plus its AABB and build digest.
/// The CPU mirror is the parity oracle; the device build uploads these same
/// bytes (see [`build_input_bytes`]).
#[derive(Clone, PartialEq, Debug)]
pub struct Blas {
    /// BLAS id; the arena shell is [`ARENA_BLAS_ID`], fighters follow.
    id: u32,
    /// Vertex positions in meters; refit rewrites these in place.
    positions: Vec<[f32; 3]>,
    /// Triangle indices; fixed at build, a change needs a rebuild.
    indices: Vec<u32>,
    /// AABB minimum corner in meters.
    aabb_min: [f32; 3],
    /// AABB maximum corner in meters.
    aabb_max: [f32; 3],
    /// Zero-area triangles kept (harmless for queries, counted for RT10).
    degenerate_tris: usize,
    /// FNV-1a64 over the build inputs; identical proxies digest identically.
    digest: u64,
    /// Successful skinned refits since build.
    refits: u64,
}

impl Blas {
    /// Build one BLAS from a validated proxy: copies the mirror, measures
    /// the AABB, counts degenerate triangles and digests the inputs.
    /// Post-validate work is a single fused scan (one vertex loop plus one
    /// triangle loop with the digest streamed inside); error precedence
    /// still comes from [`ProxyMesh::validate`], output is bit-identical.
    pub fn build(id: u32, proxy: &ProxyMesh<'_>) -> Result<Self, AccelError> {
        proxy.validate()?;
        let (aabb_min, aabb_max, degenerate_tris, digest) =
            build_scan(id, proxy.positions, proxy.indices);
        Ok(Self {
            id,
            positions: proxy.positions.to_vec(),
            indices: proxy.indices.to_vec(),
            aabb_min,
            aabb_max,
            degenerate_tris,
            digest,
            refits: 0,
        })
    }

    /// BLAS id.
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Vertex count.
    pub fn vert_count(&self) -> usize {
        self.positions.len()
    }

    /// Triangle count.
    pub fn tri_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// AABB corners in meters.
    pub fn aabb(&self) -> ([f32; 3], [f32; 3]) {
        (self.aabb_min, self.aabb_max)
    }

    /// Zero-area triangles kept at build.
    pub fn degenerate_tris(&self) -> usize {
        self.degenerate_tris
    }

    /// Build digest.
    pub fn digest(&self) -> u64 {
        self.digest
    }

    /// Successful skinned refits since build.
    pub fn refits(&self) -> u64 {
        self.refits
    }

    /// Live vertex mirror (skinning seam reads this layout to pose).
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.positions
    }

    /// Live index mirror.
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// Skinned refit: rewrite every vertex in place (same count, validated
    /// finite), remeasure the AABB and redigest. Same-count poses never
    /// reallocate; a topology change fails as [`AccelError::NeedsRebuild`]
    /// instead of silently rebuilding mid-frame. Returns the refit receipt.
    pub fn refit_positions(&mut self, positions: &[[f32; 3]]) -> Result<RefitReceipt, AccelError> {
        if positions.len() != self.positions.len() {
            return Err(AccelError::NeedsRebuild {
                built: self.positions.len(),
                got: positions.len(),
            });
        }
        for (i, p) in positions.iter().enumerate() {
            if !p.iter().all(|v| v.is_finite()) {
                return Err(AccelError::Invalid(format!(
                    "accel refit positions[{i}] is not finite"
                )));
            }
        }
        let started = Instant::now();
        self.positions.copy_from_slice(positions);
        // Fused AABB plus streamed redigest in one vertex loop; the index
        // words feed straight after in stored order. Same bytes the old
        // measure-plus-hash produced, with no temporary image.
        let mut hasher = Fnv64::new();
        hasher.write(&self.id.to_le_bytes());
        hasher.write(&(self.positions.len() as u64).to_le_bytes());
        hasher.write(&(self.indices.len() as u64).to_le_bytes());
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for p in &self.positions {
            for i in 0..3 {
                if p[i] < min[i] {
                    min[i] = p[i];
                }
                if p[i] > max[i] {
                    max[i] = p[i];
                }
                hasher.write(&p[i].to_le_bytes());
            }
        }
        for index in &self.indices {
            hasher.write(&index.to_le_bytes());
        }
        self.aabb_min = min;
        self.aabb_max = max;
        self.digest = hasher.finish();
        self.refits = self.refits.saturating_add(1);
        Ok(RefitReceipt {
            blas_id: self.id,
            verts: self.positions.len() as u64,
            tris: self.tri_count() as u64,
            refits: self.refits,
            ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Bind seam for the headless twin: gates the tier, checks the shape,
    /// then names the missing device bind. The window build next to it
    /// uploads these same bytes for real.
    #[cfg(not(feature = "window"))]
    pub fn submit_gpu(&self, tier: Tier) -> Result<RefitReceipt, AccelError> {
        require_rt(tier).map_err(|e| match e {
            AccelError::TierFallback {
                requested,
                fallback,
                reason,
                ..
            } => AccelError::TierFallback {
                requested,
                fallback,
                reason,
                adapter: "headless twin".to_owned(),
            },
            other => other,
        })?;
        self.positions
            .first()
            .ok_or_else(|| AccelError::Invalid("accel BLAS holds no vertices".into()))?;
        Err(AccelError::NoDevice(
            "accel GPU build needs the window device submitter; the CPU mirror is the reference"
                .into(),
        ))
    }
}

/// One skinned refit: vertex/triangle counts plus wall-clock ms for the frame
/// report (ms is evidence only, never hashed).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RefitReceipt {
    /// BLAS refit.
    pub blas_id: u32,
    /// Vertices rewritten.
    pub verts: u64,
    /// Triangles covered.
    pub tris: u64,
    /// Refit generation after this call.
    pub refits: u64,
    /// Wall-clock refit time in ms.
    pub ms: f64,
}

impl RefitReceipt {
    /// Posted per-frame budget this refit is charged against.
    pub fn budget_ms(&self) -> f64 {
        ACCEL_REFIT_BUDGET_MS
    }

    /// True while the refit stayed inside the posted budget.
    pub fn within_budget(&self) -> bool {
        self.ms <= ACCEL_REFIT_BUDGET_MS
    }
}

/// Deterministic build-input bytes: id, counts, every position bit, every
/// index word. Same proxy builds identical bytes twice on every run; the
/// device submit uploads this exact image.
pub fn build_input_bytes(blas: &Blas) -> Vec<u8> {
    build_input_bytes_raw(blas.id, &blas.positions, &blas.indices)
}

fn build_input_bytes_raw(id: u32, positions: &[[f32; 3]], indices: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + positions.len() * 12 + indices.len() * 4);
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&(positions.len() as u64).to_le_bytes());
    out.extend_from_slice(&(indices.len() as u64).to_le_bytes());
    for p in positions {
        for v in p {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    for index in indices {
        out.extend_from_slice(&index.to_le_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// TLAS: sorted instance list rebuilt or refit per frame.
// ---------------------------------------------------------------------------

/// Row-major 3x4 instance transform: rows of (right, up, forward, origin).
pub type InstanceTransform = [f32; 12];

/// Identity transform.
pub fn identity_transform() -> InstanceTransform {
    [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0,
    ]
}

/// Translation transform in meters.
pub fn translation_transform(offset_m: [f32; 3]) -> InstanceTransform {
    [
        1.0,
        0.0,
        0.0,
        offset_m[0], //
        0.0,
        1.0,
        0.0,
        offset_m[1], //
        0.0,
        0.0,
        1.0,
        offset_m[2],
    ]
}

/// One TLAS instance: which BLAS, where, and how the RT passes see it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TlasInstance {
    /// BLAS id referenced.
    pub blas: u32,
    /// Row-major 3x4 world transform, finite.
    pub transform: InstanceTransform,
    /// Custom index the shaders read (fighter slot or arena tag).
    pub custom_index: u32,
    /// Visibility mask, nonzero.
    pub mask: u8,
    /// True for skinned fighters (refit every frame), false for the static
    /// arena shell (rebuilt only on hirebuild).
    pub skinned: bool,
}

impl TlasInstance {
    /// Validate the transform and mask before any TLAS write.
    pub fn validate(&self) -> Result<(), AccelError> {
        if !self.transform.iter().all(|v| v.is_finite()) {
            return Err(AccelError::Invalid(format!(
                "accel instance of BLAS {} has a non-finite transform",
                self.blas
            )));
        }
        if self.mask == 0 {
            return Err(AccelError::Invalid(format!(
                "accel instance of BLAS {} has a zero mask",
                self.blas
            )));
        }
        Ok(())
    }
}

/// One top-level structure: the sorted per-frame instance list plus its
/// digest and generation. Sorted by (blas, custom_index) in stable order so
/// submission order never decides the bytes.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Tlas {
    instances: Vec<TlasInstance>,
    digest: u64,
    generation: u64,
}

impl Tlas {
    /// Build one TLAS from instances: validates every entry, sorts stably by
    /// (blas, custom_index) and digests the sorted bytes.
    pub fn build(instances: &[TlasInstance]) -> Result<Self, AccelError> {
        if instances.len() > ACCEL_MAX_INSTANCES {
            return Err(AccelError::OverBudget {
                count: instances.len(),
                max: ACCEL_MAX_INSTANCES,
            });
        }
        for instance in instances {
            instance.validate()?;
        }
        let mut sorted = instances.to_vec();
        sorted.sort_by_key(|a| (a.blas, a.custom_index));
        let mut tlas = Self {
            instances: sorted,
            digest: 0,
            generation: 0,
        };
        tlas.digest = tlas.input_digest();
        Ok(tlas)
    }

    /// Per-frame refit: revalidate, resort and redigest into the existing
    /// allocation (capacity only grows when the instance count grows, which
    /// the WIN1 match never does mid-frame). Bumps the generation.
    pub fn refit_in_place(
        &mut self,
        instances: &[TlasInstance],
    ) -> Result<TlasReceipt, AccelError> {
        if instances.len() > ACCEL_MAX_INSTANCES {
            return Err(AccelError::OverBudget {
                count: instances.len(),
                max: ACCEL_MAX_INSTANCES,
            });
        }
        for instance in instances {
            instance.validate()?;
        }
        let started = Instant::now();
        self.instances.clear();
        self.instances.extend_from_slice(instances);
        self.instances.sort_by_key(|a| (a.blas, a.custom_index));
        self.digest = self.input_digest();
        self.generation = self.generation.saturating_add(1);
        Ok(TlasReceipt {
            instances: self.instances.len() as u64,
            generation: self.generation,
            ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Live instance list in submit order.
    pub fn instances(&self) -> &[TlasInstance] {
        &self.instances
    }

    /// TLAS digest over the sorted instance bytes.
    pub fn digest(&self) -> u64 {
        self.digest
    }

    /// Refit generation; build is zero.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// FNV-1a64 over the sorted instance bytes.
    fn input_digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(&(self.instances.len() as u64).to_le_bytes());
        for instance in &self.instances {
            hasher.write(&instance.blas.to_le_bytes());
            for v in &instance.transform {
                hasher.write(&v.to_le_bytes());
            }
            hasher.write(&instance.custom_index.to_le_bytes());
            hasher.write(&[instance.mask]);
            hasher.write(&[u8::from(instance.skinned)]);
        }
        hasher.finish()
    }
}

/// One TLAS refit: instance count, generation and wall-clock ms for the
/// frame report (ms is evidence only, never hashed).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TlasReceipt {
    /// Instances resident after the refit.
    pub instances: u64,
    /// Generation after the refit.
    pub generation: u64,
    /// Wall-clock refit time in ms.
    pub ms: f64,
}

impl TlasReceipt {
    /// Posted per-frame budget this refit is charged against.
    pub fn budget_ms(&self) -> f64 {
        ACCEL_REFIT_BUDGET_MS
    }

    /// True while the refit stayed inside the posted budget.
    pub fn within_budget(&self) -> bool {
        self.ms <= ACCEL_REFIT_BUDGET_MS
    }
}

// ---------------------------------------------------------------------------
// WIN1 match wiring: arena plus 8 fighters, TLAS per frame.
// ---------------------------------------------------------------------------

/// WIN1 per-frame instance list: the static arena shell (BLAS
/// [`ARENA_BLAS_ID`], identity, custom index 0) plus one skinned instance
/// per fighter (BLAS ids from `fighter_blas`, custom index `1 + slot`,
/// translations from `fighter_xy` in meters). Deterministic per input.
pub fn win1_frame_instances(
    fighter_blas: [u32; WIN1_FIGHTERS],
    fighter_xy: [[f32; 2]; WIN1_FIGHTERS],
) -> [TlasInstance; WIN1_INSTANCES] {
    let mut out = [TlasInstance {
        blas: ARENA_BLAS_ID,
        transform: identity_transform(),
        custom_index: 0,
        mask: 0xFF,
        skinned: false,
    }; WIN1_INSTANCES];
    for (slot, (blas, xy)) in fighter_blas.iter().zip(fighter_xy.iter()).enumerate() {
        out[slot + 1] = TlasInstance {
            blas: *blas,
            transform: translation_transform([xy[0], 0.0, xy[1]]),
            custom_index: (slot as u32) + 1,
            mask: 0xFF,
            skinned: true,
        };
    }
    out
}

/// WIN1 ring slots in meters: 8 fighters on a 6 m circle, deterministic.
pub fn win1_ring_slots() -> [[f32; 2]; WIN1_FIGHTERS] {
    [
        [6.0, 0.0],
        [4.24, 4.24],
        [0.0, 6.0],
        [-4.24, 4.24],
        [-6.0, 0.0],
        [-4.24, -4.24],
        [0.0, -6.0],
        [4.24, -4.24],
    ]
}

/// Accel graph wiring for one scene: pass names, resources and the transient
/// bytes the RA2 aliasing plan must hold. Pure data, no allocation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AccelPassPlan {
    /// Graph pass building proxy BLAS entries.
    pub build_pass: &'static str,
    /// Graph pass refitting the TLAS per frame.
    pub refit_pass: &'static str,
    /// BLAS resource the build pass writes.
    pub blas_resource: &'static str,
    /// TLAS resource the refit pass writes and the RT passes read.
    pub tlas_resource: &'static str,
    /// BLAS scratch transient bytes (sum over proxy triangles).
    pub blas_bytes: u64,
    /// TLAS descriptor transient bytes (per-instance words).
    pub tlas_bytes: u64,
}

/// Validate the proxy sizes plus the instance count, then describe the two
/// graph passes plus the bytes the RA2 aliasing plan must hold. WIN1 wires
/// `build` -> `refit` -> `forward` with the forward pass declaring
/// `.input("tlas")`; RA2 owns order and aliasing.
pub fn accel_pass_plan(proxy_tris: &[u32], instances: usize) -> Result<AccelPassPlan, AccelError> {
    if proxy_blas_count(proxy_tris.len()) > ACCEL_MAX_BLAS {
        return Err(AccelError::OverBudget {
            count: proxy_tris.len(),
            max: ACCEL_MAX_BLAS,
        });
    }
    if instances > ACCEL_MAX_INSTANCES {
        return Err(AccelError::OverBudget {
            count: instances,
            max: ACCEL_MAX_INSTANCES,
        });
    }
    let mut blas_bytes = 0u64;
    for tris in proxy_tris {
        if *tris == 0 || u64::from(*tris) > u64::from(ACCEL_MAX_PROXY_TRIS) {
            return Err(AccelError::Invalid(format!(
                "accel proxy holds {tris} tris, want 1..={}",
                ACCEL_MAX_PROXY_TRIS
            )));
        }
        blas_bytes = blas_bytes
            .saturating_add(u64::from(*tris).saturating_mul(ACCEL_BLAS_SCRATCH_PER_TRI_BYTES));
    }
    Ok(AccelPassPlan {
        build_pass: ACCEL_BUILD_PASS,
        refit_pass: ACCEL_REFIT_PASS,
        blas_resource: ACCEL_BLAS_RESOURCE,
        tlas_resource: ACCEL_TLAS_RESOURCE,
        blas_bytes,
        tlas_bytes: instances as u64 * ACCEL_TLAS_INSTANCE_BYTES,
    })
}

fn proxy_blas_count(len: usize) -> usize {
    len
}

// ---------------------------------------------------------------------------
// Window GPU path: build-input validation compute over the BLAS/TLAS bytes.
// Engine-owned WGSL, no donor code. This stage proves the exact bytes the
// later ray-query passes will bind: every triangle finite and inside its
// BLAS AABB, every instance transform finite with a nonzero mask. Full
// BLAS/TLAS object construction rides the wgpu ray-query surface in RT2.
// ---------------------------------------------------------------------------

/// Byte size of the packed accel validation uniform: triangle count, tri
/// stride, instance base in f32 words, instance count (16-byte aligned).
#[cfg(feature = "window")]
pub const ACCEL_UNIFORM_BYTES: u64 = 16;

/// Words per triangle record: 3 verts plus the BLAS AABB corners.
#[cfg(feature = "window")]
pub const ACCEL_TRI_WORDS: usize = 15;
/// Words per instance record: 12 transform words plus custom, mask, blas.
#[cfg(feature = "window")]
pub const ACCEL_INSTANCE_WORDS: usize = 16;

/// Accel validation compute: one thread per record. Triangle threads check
/// finite vertices inside the BLAS AABB; instance threads check a finite
/// transform with a nonzero mask. Flag words read 1 on pass, 0 on fail.
#[cfg(feature = "window")]
const ACCEL_VALIDATE_WGSL: &str = r#"
struct AccelU {
    tri_count: u32,
    tri_stride: u32,
    inst_base_words: u32,
    inst_count: u32,
};
@group(0) @binding(0) var<uniform> cfg: AccelU;
@group(0) @binding(1) var<storage, read> recs: array<f32>;
@group(0) @binding(2) var<storage, read_write> flags: array<u32>;
fn finite3(x: f32, y: f32, z: f32) -> bool {
    return !(x != x || y != y || z != z)
        && abs(x) < 3.4028235e+38 && abs(y) < 3.4028235e+38 && abs(z) < 3.4028235e+38;
}
@compute @workgroup_size(64)
fn validate_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let total = cfg.tri_count + cfg.inst_count;
    if (i >= total) { return; }
    if (i < cfg.tri_count) {
        let base = i * cfg.tri_stride;
        let v0x = recs[base + 0u]; let v0y = recs[base + 1u]; let v0z = recs[base + 2u];
        let v1x = recs[base + 3u]; let v1y = recs[base + 4u]; let v1z = recs[base + 5u];
        let v2x = recs[base + 6u]; let v2y = recs[base + 7u]; let v2z = recs[base + 8u];
        let mnx = recs[base + 9u]; let mny = recs[base + 10u]; let mnz = recs[base + 11u];
        let mxx = recs[base + 12u]; let mxy = recs[base + 13u]; let mxz = recs[base + 14u];
        var ok = 1u;
        if (!finite3(v0x, v0y, v0z) || !finite3(v1x, v1y, v1z) || !finite3(v2x, v2y, v2z)) { ok = 0u; }
        if (!finite3(mnx, mny, mnz) || !finite3(mxx, mxy, mxz)) { ok = 0u; }
        if (mnx > mxx || mny > mxy || mnz > mxz) { ok = 0u; }
        let eps = 0.001;
        if (v0x < mnx - eps || v0y < mny - eps || v0z < mnz - eps) { ok = 0u; }
        if (v0x > mxx + eps || v0y > mxy + eps || v0z > mxz + eps) { ok = 0u; }
        if (v1x < mnx - eps || v1y < mny - eps || v1z < mnz - eps) { ok = 0u; }
        if (v1x > mxx + eps || v1y > mxy + eps || v1z > mxz + eps) { ok = 0u; }
        if (v2x < mnx - eps || v2y < mny - eps || v2z < mnz - eps) { ok = 0u; }
        if (v2x > mxx + eps || v2y > mxy + eps || v2z > mxz + eps) { ok = 0u; }
        flags[i] = ok;
    } else {
        let k = i - cfg.tri_count;
        let base = cfg.inst_base_words + k * 16u;
        var ok = 1u;
        for (var c = 0u; c < 12u; c = c + 1u) {
            let v = recs[base + c];
            if (v != v || abs(v) >= 3.4028235e+38) { ok = 0u; }
        }
        if (recs[base + 13u] < 1.0) { ok = 0u; }
        flags[i] = ok;
    }
}
"#;

/// Pack the device validation records: per-triangle verts plus BLAS AABB,
/// then per-instance transform words plus fields. Deterministic per input;
/// the CPU parity oracle ([`cpu_validate_records`]) reads this same image.
#[cfg(feature = "window")]
pub fn device_record_words(blas: &[Blas], tlas: &Tlas) -> Vec<f32> {
    let mut out = Vec::new();
    for b in blas {
        for tri in b.indices.chunks_exact(3) {
            for corner in tri {
                let p = b.positions[*corner as usize];
                out.extend_from_slice(&p);
            }
            out.extend_from_slice(&b.aabb_min);
            out.extend_from_slice(&b.aabb_max);
        }
    }
    for instance in tlas.instances() {
        out.extend_from_slice(&instance.transform);
        out.push(instance.custom_index as f32);
        out.push(f32::from(instance.mask));
        out.push(instance.blas as f32);
        out.push(0.0);
    }
    out
}

/// CPU parity oracle over [`device_record_words`]: one flag per record, 1 on
/// pass, 0 on fail. The device submit must return these exact flags.
#[cfg(feature = "window")]
pub fn cpu_validate_records(words: &[f32], tri_count: usize, inst_count: usize) -> Vec<u32> {
    let mut flags = Vec::with_capacity(tri_count + inst_count);
    for t in 0..tri_count {
        let base = t * ACCEL_TRI_WORDS;
        let rec = &words[base..base + ACCEL_TRI_WORDS];
        let mut ok = rec.iter().all(|v| v.is_finite());
        let (mn, mx) = (&rec[9..12], &rec[12..15]);
        if mn[0] > mx[0] || mn[1] > mx[1] || mn[2] > mx[2] {
            ok = false;
        }
        for corner in rec[..9].chunks_exact(3) {
            for (i, v) in corner.iter().enumerate() {
                if *v < mn[i] - ACCEL_AABB_EPS_M || *v > mx[i] + ACCEL_AABB_EPS_M {
                    ok = false;
                }
            }
        }
        flags.push(u32::from(ok));
    }
    let base_words = tri_count * ACCEL_TRI_WORDS;
    for k in 0..inst_count {
        let base = base_words + k * ACCEL_INSTANCE_WORDS;
        let rec = &words[base..base + ACCEL_INSTANCE_WORDS];
        let mut ok = rec[..12].iter().all(|v| v.is_finite());
        if rec[13] < 1.0 {
            ok = false;
        }
        flags.push(u32::from(ok));
    }
    flags
}

/// Window submit failures. There is no `Unsupported` variant: the caller
/// hands over a wgpu device, so every failure names the bad input, the
/// device call or the validation scope outcome.
#[cfg(feature = "window")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccelSubmitError {
    /// Caller input is unusable (Base tier, empty scene, over-budget
    /// records); reason carried.
    Invalid(String),
    /// A device or queue call failed (readback map or poll); message carried.
    Device(String),
    /// A wgpu validation error fired inside the submit scope, or a record
    /// flag came back failing; kept loud with the detail.
    Validation(String),
}

#[cfg(feature = "window")]
impl fmt::Display for AccelSubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(detail) => write!(f, "accel submit input: {detail}"),
            Self::Device(detail) => write!(f, "accel submit device: {detail}"),
            Self::Validation(detail) => write!(f, "accel submit validation: {detail}"),
        }
    }
}

#[cfg(feature = "window")]
impl std::error::Error for AccelSubmitError {}

/// Map the tier gate into the submit error: Base falls back loudly here too,
/// never as a silent downgrade and never as `Unsupported`.
#[cfg(feature = "window")]
fn gate_submit_tier(tier: Tier) -> Result<(), AccelSubmitError> {
    require_rt(tier).map_err(|e| AccelSubmitError::Invalid(e.to_string()))
}

/// One device accel submit: record counts, the validation outcome and the
/// wall-clock encode plus submit plus readback ms (evidence only).
#[cfg(feature = "window")]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AccelGpuReceipt {
    /// BLAS entries validated.
    pub blas_count: u64,
    /// Triangle records validated.
    pub tri_count: u64,
    /// Instance records validated.
    pub instance_count: u64,
    /// wgpu validation errors observed; green means 0.
    pub validation_errors: u32,
    /// Wall time for encode plus submit plus readback in ms (evidence only).
    pub submit_ms: f64,
}

#[cfg(feature = "window")]
impl AccelGpuReceipt {
    /// Posted per-frame budget this submit is charged against.
    pub fn budget_ms(&self) -> f64 {
        ACCEL_REFIT_BUDGET_MS
    }

    /// True while the submit stayed inside the posted budget.
    pub fn within_budget(&self) -> bool {
        self.submit_ms <= ACCEL_REFIT_BUDGET_MS
    }

    /// True when the submit observed zero validation errors.
    pub fn is_clean(&self) -> bool {
        self.validation_errors == 0
    }
}

/// Reusable device state for the accel validation pass: one compute pipeline
/// plus its bind layout and uniform buffer. Built once per frame loop (WIN1
/// and ED1 each own one); steady-state submits on any scene create no
/// pipelines (the reuse test proves the submit count stays serviced by the
/// one pipeline). Per-submit record and flag buffers are driver work, same
/// as the RA1 twin, and ride the RA2 aliasing pool once RT2 binds them.
#[cfg(feature = "window")]
pub struct AccelSubmitter {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    uniform_buf: wgpu::Buffer,
}

#[cfg(feature = "window")]
impl AccelSubmitter {
    /// Compile the validation pipeline plus the bind layout; allocate the
    /// 16-byte uniform buffer. No scene-size state: record buffers size per
    /// submit as driver work.
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("accel-validate"),
            source: wgpu::ShaderSource::Wgsl(ACCEL_VALIDATE_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("accel-validate-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("accel-validate-pipe-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("accel-validate-pipeline"),
            layout: Some(&pipe_layout),
            module: &shader,
            entry_point: Some("validate_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("accel-validate-uniform"),
            size: ACCEL_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            uniform_buf,
        }
    }

    /// Validate the BLAS/TLAS build bytes on the device and read the flags
    /// back. Gates the tier first (Base falls back loudly), validates every
    /// input before touching the device, then compares every device flag
    /// against the CPU oracle bit for bit. Fails loudly (never silent,
    /// never `Unsupported`) on bad inputs, flag mismatches or device faults.
    pub fn submit(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        tier: Tier,
        blas: &[Blas],
        tlas: &Tlas,
    ) -> Result<AccelGpuReceipt, AccelSubmitError> {
        gate_submit_tier(tier)?;
        if blas.is_empty() {
            return Err(AccelSubmitError::Invalid(
                "accel submit holds no BLAS entries".into(),
            ));
        }
        if blas.len() > ACCEL_MAX_BLAS {
            return Err(AccelSubmitError::Invalid(format!(
                "accel submit holds {} BLAS over {ACCEL_MAX_BLAS}",
                blas.len()
            )));
        }
        if tlas.instances().is_empty() {
            return Err(AccelSubmitError::Invalid(
                "accel submit holds no TLAS instances".into(),
            ));
        }
        let tri_count: usize = blas.iter().map(Blas::tri_count).sum();
        let inst_count = tlas.instances().len();
        let records = device_record_words(blas, tlas);
        let expect_records = tri_count * ACCEL_TRI_WORDS + inst_count * ACCEL_INSTANCE_WORDS;
        if records.len() != expect_records {
            return Err(AccelSubmitError::Invalid(format!(
                "accel record image {} words mismatches {expect_records}",
                records.len()
            )));
        }
        let total = tri_count + inst_count;
        let tri_u32 = u32::try_from(tri_count)
            .map_err(|_| AccelSubmitError::Invalid("accel tri count exceeds u32".into()))?;
        let inst_u32 = u32::try_from(inst_count)
            .map_err(|_| AccelSubmitError::Invalid("accel instance count exceeds u32".into()))?;
        let inst_base = u32::try_from(tri_count * ACCEL_TRI_WORDS)
            .map_err(|_| AccelSubmitError::Invalid("accel record base exceeds u32".into()))?;
        let rec_bytes = records.len() as u64 * 4;
        let flag_bytes = total as u64 * 4;
        let started = Instant::now();

        let mut rec_raw = Vec::with_capacity(records.len() * 4);
        for w in &records {
            rec_raw.extend_from_slice(&w.to_le_bytes());
        }
        let rec_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("accel-validate-recs"),
            size: rec_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let flag_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("accel-validate-flags"),
            size: flag_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("accel-validate-readback"),
            size: flag_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("accel-validate-bind"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: rec_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: flag_buf.as_entire_binding(),
                },
            ],
        });
        let mut uniform = [0u8; 16];
        uniform[0..4].copy_from_slice(&tri_u32.to_le_bytes());
        uniform[4..8].copy_from_slice(&(ACCEL_TRI_WORDS as u32).to_le_bytes());
        uniform[8..12].copy_from_slice(&inst_base.to_le_bytes());
        uniform[12..16].copy_from_slice(&inst_u32.to_le_bytes());
        queue.write_buffer(&self.uniform_buf, 0, &uniform);
        queue.write_buffer(&rec_buf, 0, &rec_raw);

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("accel-validate"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("accel-validate-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(total.div_ceil(64) as u32, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&flag_buf, 0, &readback, 0, flag_bytes);
        queue.submit(Some(encoder.finish()));
        map_readback(device, &readback).map_err(AccelSubmitError::Device)?;
        let flags = readback_u32(&readback, total).map_err(AccelSubmitError::Device)?;
        readback.unmap();
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(AccelSubmitError::Validation(format!("{err:?}")));
        }
        let expected = cpu_validate_records(&records, tri_count, inst_count);
        for (i, (got, want)) in flags.iter().zip(expected.iter()).enumerate() {
            if got != want {
                return Err(AccelSubmitError::Validation(format!(
                    "accel record {i} device flag {got} mismatches oracle {want}"
                )));
            }
            if *got != 1 {
                return Err(AccelSubmitError::Validation(format!(
                    "accel record {i} failed validation on device and oracle"
                )));
            }
        }
        Ok(AccelGpuReceipt {
            blas_count: blas.len() as u64,
            tri_count: tri_count as u64,
            instance_count: inst_count as u64,
            validation_errors: 0,
            submit_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

/// Map a readback buffer and wait for it. One poll covers every pending map.
#[cfg(feature = "window")]
fn map_readback(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Result<(), String> {
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| format!("accel poll: {e}"))?;
    rx.recv()
        .map_err(|e| format!("accel map channel: {e}"))?
        .map_err(|e| format!("accel map: {e}"))?;
    Ok(())
}

/// Read `count` little-endian u32 values from a mapped readback buffer.
#[cfg(feature = "window")]
fn readback_u32(buffer: &wgpu::Buffer, count: usize) -> Result<Vec<u32>, String> {
    let mapped = buffer
        .slice(..)
        .get_mapped_range()
        .map_err(|e| format!("accel mapped range: {e}"))?;
    if mapped.len() < count * 4 {
        return Err(format!(
            "accel readback {} bytes holds fewer than {count} u32 flags",
            mapped.len()
        ));
    }
    let mut out = Vec::with_capacity(count);
    for chunk in mapped[..count * 4].chunks_exact(4) {
        out.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unit cube shell: 8 verts, 12 tris.
    fn cube_positions() -> Vec<[f32; 3]> {
        vec![
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ]
    }

    fn cube_indices() -> Vec<u32> {
        vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5, 0, 1, 5, 0, 5,
            4, 3, 7, 6, 3, 6, 2,
        ]
    }

    fn cube_proxy<'a>(positions: &'a [[f32; 3]], indices: &'a [u32]) -> ProxyMesh<'a> {
        ProxyMesh { positions, indices }
    }

    /// Grid proxy on the unit square: `n` cells per side, row-major.
    fn grid_proxy(n: usize) -> (Vec<[f32; 3]>, Vec<u32>) {
        let mut positions = Vec::new();
        for j in 0..=n {
            for i in 0..=n {
                positions.push([i as f32 / n as f32, 0.0, j as f32 / n as f32]);
            }
        }
        let mut indices = Vec::new();
        for j in 0..n {
            for i in 0..n {
                let a = (j * (n + 1) + i) as u32;
                let b = a + 1;
                let c = a + n as u32 + 1;
                let d = c + 1;
                indices.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
        (positions, indices)
    }

    #[test]
    fn proxy_validation_names_bad_input() {
        let positions = cube_positions();
        let indices = cube_indices();
        cube_proxy(&positions, &indices)
            .validate()
            .expect("cube proxy validates");
        // Empty vertices.
        match (ProxyMesh {
            positions: &[],
            indices: &indices,
        })
        .validate()
        {
            Err(AccelError::Invalid(detail)) => assert!(detail.contains("vertices")),
            other => panic!("empty verts must fail Invalid, got {other:?}"),
        }
        // Non-multiple index length.
        match (ProxyMesh {
            positions: &positions,
            indices: &indices[..10],
        })
        .validate()
        {
            Err(AccelError::Invalid(detail)) => assert!(detail.contains("triangle")),
            other => panic!("ragged indices must fail Invalid, got {other:?}"),
        }
        // Out-of-range index names the culprit.
        let mut bad_indices = indices.clone();
        bad_indices[4] = 99;
        match cube_proxy(&positions, &bad_indices).validate() {
            Err(AccelError::Invalid(detail)) => assert!(detail.contains("99"), "{detail}"),
            other => panic!("oob index must fail Invalid, got {other:?}"),
        }
        // Non-finite vertex names the lane.
        let mut bad_positions = positions.clone();
        bad_positions[2][0] = f32::NAN;
        match cube_proxy(&bad_positions, &indices).validate() {
            Err(AccelError::Invalid(detail)) => {
                assert!(detail.contains("positions[2]"), "{detail}");
            }
            other => panic!("NaN vertex must fail Invalid, got {other:?}"),
        }
        // All zero-area is degenerate, never a hollow BLAS.
        let flat = vec![[0.0, 0.0, 0.0]; 4];
        let flat_indices = vec![0, 1, 2, 0, 2, 3];
        match (ProxyMesh {
            positions: &flat,
            indices: &flat_indices,
        })
        .validate()
        {
            Err(AccelError::Degenerate(detail)) => assert!(detail.contains("zero-area")),
            other => panic!("flat proxy must fail Degenerate, got {other:?}"),
        }
    }

    #[test]
    fn oversized_proxy_refused_with_counts() {
        // Index budget trips first on a fake-large triangle list.
        let positions = cube_positions();
        let big: Vec<u32> = (0..(ACCEL_MAX_PROXY_TRIS + 1) as usize)
            .flat_map(|_| [0u32, 1u32, 2u32])
            .collect();
        match (ProxyMesh {
            positions: &positions,
            indices: &big,
        })
        .validate()
        {
            Err(AccelError::OverBudget { count, max }) => {
                assert_eq!((count, max), (big.len() / 3, ACCEL_MAX_PROXY_TRIS as usize));
            }
            other => panic!("oversized proxy must fail OverBudget, got {other:?}"),
        }
    }

    #[test]
    fn blas_build_measures_aabb_and_digests() {
        let positions = cube_positions();
        let indices = cube_indices();
        let blas = Blas::build(3, &cube_proxy(&positions, &indices)).expect("cube builds");
        assert_eq!((blas.id(), blas.vert_count(), blas.tri_count()), (3, 8, 12));
        assert_eq!(blas.aabb(), ([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]));
        assert_eq!(blas.degenerate_tris(), 0);
        assert_eq!(blas.refits(), 0);
        assert_ne!(blas.digest(), 0, "digest must not be the zero hash");
    }

    #[test]
    fn build_inputs_identical_twice() {
        // Merge gate: same proxy bytes build identical input bytes twice.
        let positions = cube_positions();
        let indices = cube_indices();
        let first = Blas::build(1, &cube_proxy(&positions, &indices)).expect("first builds");
        let second = Blas::build(1, &cube_proxy(&positions, &indices)).expect("second builds");
        assert_eq!(build_input_bytes(&first), build_input_bytes(&second));
        assert_eq!(first.digest(), second.digest());
        assert_eq!(first, second);
    }

    #[test]
    fn partly_degenerate_proxy_builds_with_count() {
        // One collapsed corner triangle stays counted for the RT10 gate.
        let mut positions = cube_positions();
        positions[7] = positions[6];
        let indices = cube_indices();
        let blas = Blas::build(0, &cube_proxy(&positions, &indices)).expect("builds");
        assert!(
            blas.degenerate_tris() >= 1,
            "collapsed corner must count, got {}",
            blas.degenerate_tris()
        );
    }

    #[test]
    fn skinned_refit_rewrites_aabb_and_digest() {
        let positions = cube_positions();
        let indices = cube_indices();
        let mut blas = Blas::build(2, &cube_proxy(&positions, &indices)).expect("builds");
        let before = blas.digest();
        // Worst case: every vertex moves (full pose update every frame).
        let posed: Vec<[f32; 3]> = positions
            .iter()
            .map(|p| [p[0] + 0.5, p[1] - 0.25, p[2] + 0.125])
            .collect();
        let receipt = blas.refit_positions(&posed).expect("refit applies");
        assert_eq!((receipt.blas_id, receipt.refits), (2, 1));
        assert_eq!((receipt.verts, receipt.tris), (8, 12));
        assert_ne!(blas.digest(), before, "moved verts must redigest");
        assert_eq!(blas.aabb(), ([-0.5, -1.25, -0.875], [1.5, 0.75, 1.125]));
        assert!(receipt.within_budget(), "8-vert refit must clear 1.5 ms");
        assert_eq!(receipt.budget_ms(), ACCEL_REFIT_BUDGET_MS);
        // Reposing back to the bind pose restores the build digest exactly.
        blas.refit_positions(&positions)
            .expect("bind pose restores");
        assert_eq!(blas.digest(), before);
    }

    #[test]
    fn skinned_refit_failures_are_loud() {
        let positions = cube_positions();
        let indices = cube_indices();
        let mut blas = Blas::build(2, &cube_proxy(&positions, &indices)).expect("builds");
        // Topology change needs a rebuild, never a silent realloc mid-frame.
        match blas.refit_positions(&positions[..7]) {
            Err(AccelError::NeedsRebuild { built, got }) => assert_eq!((built, got), (8, 7)),
            other => panic!("topology change must fail NeedsRebuild, got {other:?}"),
        }
        // NaN pose is refused and keeps the old mirror intact.
        let mut bad = positions.clone();
        bad[0][1] = f32::INFINITY;
        match blas.refit_positions(&bad) {
            Err(AccelError::Invalid(detail)) => assert!(detail.contains("positions[0]")),
            other => panic!("non-finite pose must fail Invalid, got {other:?}"),
        }
        assert_eq!(blas.refits(), 0, "failed refits never bump the generation");
        assert_eq!(blas.positions(), positions.as_slice());
    }

    #[test]
    fn skinned_refit_reuses_every_allocation() {
        // Steady-state proof: 60 worst-case refits never grow the mirror.
        let positions = cube_positions();
        let indices = cube_indices();
        let mut blas = Blas::build(2, &cube_proxy(&positions, &indices)).expect("builds");
        let (pos_cap, idx_cap) = (blas.positions.capacity(), blas.indices.capacity());
        for frame in 0..60 {
            let posed: Vec<[f32; 3]> = positions
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let push = (((frame * 7 + i * 13) % 17) as f32) * 0.001;
                    [p[0] + push, p[1] - push, p[2] + push]
                })
                .collect();
            blas.refit_positions(&posed).expect("frame refits");
        }
        assert_eq!(blas.refits(), 60);
        assert_eq!(
            (blas.positions.capacity(), blas.indices.capacity()),
            (pos_cap, idx_cap),
            "steady-state refit must not reallocate"
        );
    }

    #[test]
    fn tlas_build_sorts_stably_and_digests_twice() {
        let ring = win1_ring_slots();
        let fighter_blas = [1, 2, 3, 4, 5, 6, 7, 8];
        let instances = win1_frame_instances(fighter_blas, ring);
        let first = Tlas::build(&instances).expect("tlas builds");
        let second = Tlas::build(&instances).expect("tlas rebuilds");
        assert_eq!(first.digest(), second.digest());
        assert_eq!(first, second);
        // Submit order is canonical: arena first, fighters by custom index.
        let order: Vec<(u32, u32)> = first
            .instances()
            .iter()
            .map(|i| (i.blas, i.custom_index))
            .collect();
        assert_eq!(
            order,
            vec![
                (0, 0),
                (1, 1),
                (2, 2),
                (3, 3),
                (4, 4),
                (5, 5),
                (6, 6),
                (7, 7),
                (8, 8)
            ]
        );
        assert_eq!(first.generation(), 0);
    }

    #[test]
    fn tlas_refit_bumps_generation_and_reuses_allocation() {
        let ring = win1_ring_slots();
        let fighter_blas = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut tlas = Tlas::build(&win1_frame_instances(fighter_blas, ring)).expect("builds");
        let cap = tlas.instances.capacity();
        // Shuffled input still lands in canonical order with a new digest.
        let mut moved = win1_frame_instances(fighter_blas, ring);
        moved.swap(0, 8);
        moved[3].transform = translation_transform([1.0, 0.0, -2.0]);
        let receipt = tlas.refit_in_place(&moved).expect("refits");
        assert_eq!((receipt.instances, receipt.generation), (9, 1));
        assert_eq!(tlas.generation(), 1);
        assert!(
            receipt.within_budget(),
            "9-instance refit must clear 1.5 ms"
        );
        let order: Vec<u32> = tlas.instances().iter().map(|i| i.custom_index).collect();
        assert_eq!(order, (0..9).collect::<Vec<u32>>());
        for _ in 0..60 {
            tlas.refit_in_place(&moved).expect("frame refits");
        }
        assert_eq!(tlas.generation(), 61);
        assert_eq!(
            tlas.instances.capacity(),
            cap,
            "steady-state TLAS refit must not reallocate"
        );
    }

    #[test]
    fn tlas_failures_are_loud() {
        let ring = win1_ring_slots();
        let fighter_blas = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut instances = win1_frame_instances(fighter_blas, ring);
        instances[4].mask = 0;
        match Tlas::build(&instances) {
            Err(AccelError::Invalid(detail)) => assert!(detail.contains("zero mask")),
            other => panic!("zero mask must fail Invalid, got {other:?}"),
        }
        instances[4].mask = 0xFF;
        instances[4].transform[2] = f32::NAN;
        match Tlas::build(&instances) {
            Err(AccelError::Invalid(detail)) => assert!(detail.contains("non-finite")),
            other => panic!("NaN transform must fail Invalid, got {other:?}"),
        }
        let many = vec![
            TlasInstance {
                blas: 0,
                transform: identity_transform(),
                custom_index: 0,
                mask: 1,
                skinned: false,
            };
            ACCEL_MAX_INSTANCES + 1
        ];
        match Tlas::build(&many) {
            Err(AccelError::OverBudget { count, max }) => {
                assert_eq!((count, max), (ACCEL_MAX_INSTANCES + 1, ACCEL_MAX_INSTANCES));
            }
            other => panic!("instance flood must fail OverBudget, got {other:?}"),
        }
    }

    #[test]
    fn tier_gate_falls_back_loudly_never_silently() {
        // Base never runs the RT path: loud fallback naming the cascade path.
        match require_rt(Tier::Base) {
            Err(AccelError::TierFallback { reason, .. }) => {
                assert!(reason.contains("RL1"), "names the fallback path: {reason}");
            }
            other => panic!("Base must fall back loudly, got {other:?}"),
        }
        require_rt(Tier::Rt).expect("Rt runs the RT path");
        require_rt(Tier::Ultra).expect("Ultra runs the RT path");
        // Forced RT on llvmpipe is refused loudly with adapter plus reason.
        match resolve_for_match(&AdapterCaps::llvmpipe(), Some(Tier::Rt)) {
            Err(AccelError::TierFallback {
                requested,
                fallback,
                reason,
                adapter,
            }) => {
                assert_eq!((requested, fallback), (Tier::Rt, Tier::Base));
                assert!(
                    reason.contains("RAY_QUERY"),
                    "names the capability: {reason}"
                );
                assert!(adapter.contains("llvmpipe"), "names the adapter: {adapter}");
            }
            other => panic!("forced RT on llvmpipe must refuse, got {other:?}"),
        }
        // Probing still works everywhere: llvmpipe matches Base, the
        // reference GPU matches Rt, unknown labels fail loudly.
        assert_eq!(
            resolve_for_match(&AdapterCaps::llvmpipe(), None).expect("probes"),
            Tier::Base
        );
        assert_eq!(
            resolve_for_match(&AdapterCaps::reference_rt(), None).expect("probes"),
            Tier::Rt
        );
        assert!(matches!(
            resolve_for_match(&AdapterCaps::llvmpipe(), Some(Tier::Rt)),
            Err(AccelError::TierFallback { .. })
        ));
        match crate::tier::resolve_name(&AdapterCaps::llvmpipe(), Some("extreme")) {
            Err(_) => {}
            Ok(_) => panic!("unknown tier label must refuse"),
        }
    }

    #[test]
    #[cfg(not(feature = "window"))]
    fn headless_twin_names_the_device_bind() {
        let positions = cube_positions();
        let indices = cube_indices();
        let blas = Blas::build(0, &cube_proxy(&positions, &indices)).expect("builds");
        // RT tiers reach the device refusal; Base is refused at the gate.
        match blas.submit_gpu(Tier::Rt) {
            Err(AccelError::NoDevice(detail)) => {
                assert!(
                    detail.contains("window"),
                    "names the missing path: {detail}"
                );
            }
            other => panic!("headless RT submit must name the device, got {other:?}"),
        }
        match blas.submit_gpu(Tier::Base) {
            Err(AccelError::TierFallback { .. }) => {}
            other => panic!("headless Base submit must fall back loudly, got {other:?}"),
        }
    }

    /// WIN1 match frame: arena plus 8 skinned fighters, TLAS per frame.
    /// This is the consumer shape: build once, then refit every frame with
    /// worst-case full-pose skinning updates on all fighters. Exactness and
    /// wiring stay default; wall-clock budgets live in
    /// win1_match_refit_wall_budget (--perf only, load-dependent).
    #[test]
    fn win1_match_refit_within_budget() {
        let (arena_positions, arena_indices) = grid_proxy(16);
        let mut fighter_meshes: Vec<(Vec<[f32; 3]>, Vec<u32>)> = Vec::new();
        for _ in 0..WIN1_FIGHTERS {
            fighter_meshes.push(grid_proxy(8));
        }
        let mut blas_vec: Vec<Blas> = Vec::with_capacity(WIN1_INSTANCES);
        blas_vec.push(
            Blas::build(
                ARENA_BLAS_ID,
                &ProxyMesh {
                    positions: &arena_positions,
                    indices: &arena_indices,
                },
            )
            .expect("arena BLAS builds"),
        );
        for (slot, (positions, indices)) in fighter_meshes.iter().enumerate() {
            blas_vec.push(
                Blas::build((slot as u32) + 1, &ProxyMesh { positions, indices })
                    .expect("fighter BLAS builds"),
            );
        }
        assert_eq!(blas_vec.len(), WIN1_INSTANCES);
        let fighter_blas: [u32; WIN1_FIGHTERS] = [1, 2, 3, 4, 5, 6, 7, 8];
        let ring = win1_ring_slots();
        let mut tlas = Tlas::build(&win1_frame_instances(fighter_blas, ring)).expect("builds");
        // 60 frames: every fighter reposes fully, TLAS refits, budget holds.
        let mut worst_ms = 0.0f64;
        for frame in 0..60 {
            for (slot, blas) in blas_vec.iter_mut().enumerate().skip(1) {
                let base = &fighter_meshes[slot - 1].0;
                let posed: Vec<[f32; 3]> = base
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let push = (((frame * 7 + i * 13 + slot * 29) % 17) as f32) * 0.002;
                        [p[0] + push, p[1] - push, p[2] + push]
                    })
                    .collect();
                let receipt = blas.refit_positions(&posed).expect("fighter refits");
                worst_ms = worst_ms.max(receipt.ms);
            }
            let receipt = tlas
                .refit_in_place(&win1_frame_instances(fighter_blas, ring))
                .expect("tlas refits");
            worst_ms = worst_ms.max(receipt.ms);
        }
        assert_eq!(tlas.generation(), 60);
        assert_eq!(tlas.instances().len(), WIN1_INSTANCES);
        eprintln!(
            "RT1 WIN1 refit: arena {} tris plus 8x{} tris, worst frame refit {worst_ms:.3} ms \
             vs {ACCEL_REFIT_BUDGET_MS} ms budget (budget asserted in --perf only)",
            blas_vec[0].tri_count(),
            blas_vec[1].tri_count(),
        );
    }

    #[test]
    #[ignore = "timing: refit wall-clock needs release --perf on the idle reference GPU (fails under parallel load)"]
    fn win1_match_refit_wall_budget() {
        let (arena_positions, arena_indices) = grid_proxy(16);
        let mut fighter_meshes: Vec<(Vec<[f32; 3]>, Vec<u32>)> = Vec::new();
        for _ in 0..WIN1_FIGHTERS {
            fighter_meshes.push(grid_proxy(8));
        }
        let mut blas_vec: Vec<Blas> = Vec::with_capacity(WIN1_INSTANCES);
        blas_vec.push(
            Blas::build(
                ARENA_BLAS_ID,
                &ProxyMesh {
                    positions: &arena_positions,
                    indices: &arena_indices,
                },
            )
            .expect("arena BLAS builds"),
        );
        for (slot, (positions, indices)) in fighter_meshes.iter().enumerate() {
            blas_vec.push(
                Blas::build((slot as u32) + 1, &ProxyMesh { positions, indices })
                    .expect("fighter BLAS builds"),
            );
        }
        let fighter_blas: [u32; WIN1_FIGHTERS] = [1, 2, 3, 4, 5, 6, 7, 8];
        let ring = win1_ring_slots();
        let mut tlas = Tlas::build(&win1_frame_instances(fighter_blas, ring)).expect("builds");
        // Perf harness lines: track the worst refit so --perf medians are readable.
        let mut worst_ms = 0.0f64;
        for frame in 0..60 {
            for (slot, blas) in blas_vec.iter_mut().enumerate().skip(1) {
                let base = &fighter_meshes[slot - 1].0;
                let posed: Vec<[f32; 3]> = base
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let push = (((frame * 7 + i * 13 + slot * 29) % 17) as f32) * 0.002;
                        [p[0] + push, p[1] - push, p[2] + push]
                    })
                    .collect();
                let receipt = blas.refit_positions(&posed).expect("fighter refits");
                worst_ms = worst_ms.max(receipt.ms);
                assert!(
                    receipt.within_budget(),
                    "fighter refit over budget: {receipt:?}"
                );
            }
            let receipt = tlas
                .refit_in_place(&win1_frame_instances(fighter_blas, ring))
                .expect("tlas refits");
            worst_ms = worst_ms.max(receipt.ms);
            assert!(
                receipt.within_budget(),
                "TLAS refit over budget: {receipt:?}"
            );
        }
        eprintln!(
            "win1_match_refit_wall_budget: worst frame refit {worst_ms:.3} ms vs {ACCEL_REFIT_BUDGET_MS} ms budget"
        );
    }

    #[test]
    fn accel_pass_plan_wires_into_the_frame_graph() {
        // WIN1 wiring shape: build writes blas, refit reads blas plus writes
        // tlas, forward reads tlas. RA2 owns order and aliasing.
        let plan = accel_pass_plan(
            &[512, 128, 128, 128, 128, 128, 128, 128, 128],
            WIN1_INSTANCES,
        )
        .expect("WIN1 plans");
        assert_eq!(
            (plan.build_pass, plan.refit_pass),
            (ACCEL_BUILD_PASS, ACCEL_REFIT_PASS)
        );
        assert_eq!(
            (plan.blas_resource, plan.tlas_resource),
            (ACCEL_BLAS_RESOURCE, ACCEL_TLAS_RESOURCE)
        );
        assert_eq!(
            plan.blas_bytes,
            (512 + 8 * 128) as u64 * ACCEL_BLAS_SCRATCH_PER_TRI_BYTES
        );
        assert_eq!(
            plan.tlas_bytes,
            WIN1_INSTANCES as u64 * ACCEL_TLAS_INSTANCE_BYTES
        );
        let mut graph = crate::graph::FrameGraph::new();
        let build = graph.add_pass(
            crate::graph::PassDesc::new(plan.build_pass)
                .output(plan.blas_resource)
                .transient(plan.blas_resource, plan.blas_bytes),
        );
        let cached = graph.add_pass(
            crate::graph::PassDesc::new("forward-cache")
                .input("scene")
                .output("hdr")
                .work(8, 8),
        );
        let _ = cached;
        let refit = graph.add_pass(
            crate::graph::PassDesc::new(plan.refit_pass)
                .input(plan.blas_resource)
                .output(plan.tlas_resource)
                .transient(plan.tlas_resource, plan.tlas_bytes),
        );
        let forward = graph.add_pass(
            crate::graph::PassDesc::new("forward")
                .input(plan.tlas_resource)
                .input("hdr")
                .output("hdr")
                .work(256, 1_800_000),
        );
        graph.add_edge(build, refit).expect("dense pass ids");
        graph.add_edge(refit, forward).expect("dense pass ids");
        let mut compiled = graph.compile().expect("accel chain is acyclic");
        assert_eq!(compiled.order(), &[build, 1, refit, forward]);
        assert!(
            compiled.barriers().iter().any(|b| {
                b.resource == plan.tlas_resource
                    && b.kind == crate::graph::BarrierKind::Transition
                    && b.from == refit
                    && b.to == forward
            }),
            "refit -> forward hands over tlas"
        );
        let receipt = compiled.execute(&crate::graph::FrameInfo {
            frame_index: 9,
            width_px: 64,
            height_px: 64,
        });
        assert_eq!(receipt.draws, 264, "forward draws survive the chain");
        // Over-budget plans refuse before any graph work.
        match accel_pass_plan(&[0], WIN1_INSTANCES) {
            Err(AccelError::Invalid(_)) => {}
            other => panic!("zero-tri proxy must fail Invalid, got {other:?}"),
        }
        match accel_pass_plan(&[12], ACCEL_MAX_INSTANCES + 1) {
            Err(AccelError::OverBudget { .. }) => {}
            other => panic!("instance flood must fail OverBudget, got {other:?}"),
        }
    }

    // ----- window device tests (behind `window`; skip loudly without one) -----

    #[cfg(feature = "window")]
    fn accel_window_scene() -> (Vec<Blas>, Tlas) {
        let positions = cube_positions();
        let indices = cube_indices();
        let arena = Blas::build(
            ARENA_BLAS_ID,
            &ProxyMesh {
                positions: &positions,
                indices: &indices,
            },
        )
        .expect("arena builds");
        let fighter = Blas::build(
            1,
            &ProxyMesh {
                positions: &positions,
                indices: &indices,
            },
        )
        .expect("fighter builds");
        let tlas = Tlas::build(&win1_frame_instances(
            [1, 1, 1, 1, 1, 1, 1, 1],
            win1_ring_slots(),
        ))
        .expect("tlas builds");
        (vec![arena, fighter], tlas)
    }

    #[test]
    #[cfg(feature = "window")]
    fn accel_window_submit_validates_clean() {
        let Some((device, queue, info)) = crate::graph::gx1_test_device() else {
            eprintln!("SKIPPED accel window submit: no wgpu adapter on this machine");
            return;
        };
        let (blas, tlas) = accel_window_scene();
        let submitter = AccelSubmitter::new(device);
        // First submit warms the pipeline; the second is the steady-state
        // frame (PF2 enforces the 1.5 ms budget at arena size in release;
        // debug wall-clock here is evidence only, never the gate).
        let first = submitter
            .submit(device, queue, Tier::Rt, &blas, &tlas)
            .expect("first submit runs");
        let receipt = submitter
            .submit(device, queue, Tier::Rt, &blas, &tlas)
            .expect("accel window submit runs clean");
        assert!(receipt.is_clean(), "0 validation errors: {receipt:?}");
        assert_eq!((receipt.blas_count, receipt.instance_count), (2, 9));
        assert_eq!(receipt.tri_count, 24);
        assert!(
            first.is_clean(),
            "warmup submit must validate too: {first:?}"
        );
        // Device flags match the CPU oracle bit for bit on this scene.
        let tri_count: usize = blas.iter().map(Blas::tri_count).sum();
        let records = device_record_words(&blas, &tlas);
        assert!(
            cpu_validate_records(&records, tri_count, tlas.instances().len())
                .iter()
                .all(|f| *f == 1),
            "oracle must pass the clean scene"
        );
        eprintln!(
            "RT1 accel receipt on {}: blas={} tris={} instances={} warm_ms={:.2} steady_ms={:.2} budget_ms={}",
            info.name,
            receipt.blas_count,
            receipt.tri_count,
            receipt.instance_count,
            first.submit_ms,
            receipt.submit_ms,
            ACCEL_REFIT_BUDGET_MS,
        );
    }

    #[test]
    #[cfg(feature = "window")]
    fn accel_receipt_budget_accessors_are_exact() {
        // Deterministic budget logic (no wall clock): at, under and over
        // the posted budget.
        let at = AccelGpuReceipt {
            blas_count: 9,
            tri_count: 1536,
            instance_count: 9,
            validation_errors: 0,
            submit_ms: ACCEL_REFIT_BUDGET_MS,
        };
        assert_eq!(at.budget_ms(), ACCEL_REFIT_BUDGET_MS);
        assert!(at.within_budget(), "exactly at budget is within");
        assert!(at.is_clean(), "zero validation errors is clean");
        let over = AccelGpuReceipt {
            submit_ms: ACCEL_REFIT_BUDGET_MS + 0.5,
            ..at
        };
        assert!(!over.within_budget(), "over budget fails loudly");
        let dirty = AccelGpuReceipt {
            validation_errors: 1,
            ..at
        };
        assert!(!dirty.is_clean(), "validation errors are never clean");
    }

    #[test]
    #[cfg(feature = "window")]
    fn accel_window_base_falls_back_loudly_never_unsupported() {
        let Some((device, queue, _)) = crate::graph::gx1_test_device() else {
            eprintln!("SKIPPED accel base fallback: no wgpu adapter on this machine");
            return;
        };
        let (blas, tlas) = accel_window_scene();
        let submitter = AccelSubmitter::new(device);
        // Base on a live device still refuses the RT path loudly: the word
        // "unsupported" never appears on this path.
        match submitter.submit(device, queue, Tier::Base, &blas, &tlas) {
            Err(AccelSubmitError::Invalid(detail)) => {
                assert!(!detail.is_empty(), "never silent");
                assert!(
                    !detail.to_lowercase().contains("unsupported"),
                    "window path never reports Unsupported: {detail}"
                );
                assert!(detail.contains("RL1"), "names the fallback: {detail}");
            }
            other => panic!("Base submit must fall back loudly, got {other:?}"),
        }
        // Empty scenes fail Invalid before sizing anything.
        match submitter.submit(device, queue, Tier::Rt, &[], &tlas) {
            Err(AccelSubmitError::Invalid(detail)) => assert!(detail.contains("no BLAS")),
            other => panic!("empty BLAS must fail Invalid, got {other:?}"),
        }
    }

    #[test]
    #[cfg(feature = "window")]
    fn accel_window_oracle_catches_a_broken_record() {
        // Tamper one vertex past its AABB: oracle flags it, so the submit
        // would too. Proves the oracle is not a constant pass.
        let (blas, tlas) = accel_window_scene();
        let tri_count: usize = blas.iter().map(Blas::tri_count).sum();
        let mut records = device_record_words(&blas, &tlas);
        records[0] = 999.0;
        let flags = cpu_validate_records(&records, tri_count, tlas.instances().len());
        assert_eq!(flags[0], 0, "tampered triangle must fail the oracle");
        assert!(
            flags[1..].iter().all(|f| *f == 1),
            "untouched records must still pass"
        );
    }
}
