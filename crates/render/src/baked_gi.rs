//! baked_gi: arena lightmaps with a validity mask + fighter SH probes (RL5).
//!
//! Owner: render lane; consumers WIN1/ED1 arena frame ([`sample_arena_frame`]
//! per static texel and per fighter) and Q1 captures (fixture + digest).
//! Units: texels for the lightmap, meters for probe and sample positions, ms
//! in receipts. Determinism: fixed row-major bake order over the `gi` math;
//! same complete inputs give bit-identical bytes x2. Presentation only:
//! pixels never enter the sim hash. Errors are typed [`BakedGiError`].
//!
//! CODELAW law 1: SH lives in `ibl.rs` ([`IrradianceSh`], [`eval_sh`]) over
//! the `gi.rs` basis; this module owns the mask, the tunnel seal and the
//! frame bind, never a second SH path.

#![forbid(unsafe_code)]

pub use crate::baked_bake::{
    arena_fixture_parts, arena_floor_samples, arena_tunnel, bake_arena, bake_arena_fixture,
    ArenaBake, BakedGiError, ProbeSpec, SealedBox, ARENA_LIGHTMAP_H, ARENA_LIGHTMAP_W,
    BAKE_MAX_DIM, PROBE_MAX_AXIS, PROBE_MAX_COUNT,
};
use std::time::Instant;

use crate::gi::{f32_from_u32, palette_entry, u32_from_f32};
use crate::ibl::{eval_sh, IrradianceSh};

// ---------------------------------------------------------------------------
// Bounds: frame-side sizes validated before allocation (bake bounds live in
// `baked_bake` as the single owner).
// ---------------------------------------------------------------------------

/// Largest accepted viewport edge in pixels (mirrors the RA1 frame bound).
pub const MAX_VIEWPORT_PX: u32 = 2048;
/// Posted frame budget in ms: baked arena + 8 fighters holds 60 fps on the
/// reference GPU (PF2 enforces; the CPU lookup below is a slice of it).
pub const GI_FRAME_BUDGET_MS: f64 = 16.6;
/// Posted CPU lookup budget in ms for one 8-fighter frame on the test
/// machine (evidence only, never hashed; the reference-GPU capture owns the
/// look gate).
pub const FIGHTER_SAMPLE_BUDGET_MS: f64 = 0.5;

// Bake errors, boxes, specs, and the arena bake live in `baked_bake`
// (single owner); re-exported above for the existing `baked_gi` paths.

// Arena bake types and the bake itself live in `baked_bake` (single owner);
// re-exported above. This module keeps probe/frame/GPU binds below.

// (ArenaBake methods live in `baked_bake`; see re-exports above.)

// (Bake helpers `sample_sound`/`check_scene` live in `baked_bake`.)

// (`bake_arena` lives in `baked_bake`; re-exported above.)

// ---------------------------------------------------------------------------
// Runtime lookups: lightmap with mask fallback plus fighter probes via ibl.
// ---------------------------------------------------------------------------

/// Static lightmap fetch: byte-exact palette row while the texel mask reads
/// 1, `None` while it reads 0 (or `u`/`v` leave `0..=1`, or are non-finite).
/// `None` is the consumer fallback signal: shade from probe ambient instead.
pub fn sample_lightmap(bake: &ArenaBake, u: f32, v: f32) -> Option<[u8; 3]> {
    if bake.map.w == 0 || bake.map.h == 0 {
        return None;
    }
    if !(u.is_finite() && v.is_finite() && (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)) {
        return None;
    }
    let w = bake.map.w as usize;
    let h = bake.map.h as usize;
    let x = ((u * w as f32).floor() as usize).min(w - 1);
    let y = ((v * h as f32).floor() as usize).min(h - 1);
    let i = y * w + x;
    if bake.valid.get(i).copied().unwrap_or(0) != 1 {
        return None;
    }
    bake.map.idx.get(i).copied().map(palette_entry)
}

/// Linear HDR companion of [`sample_lightmap`]: the RGB9E5 lane at the same
/// texel, decoded to linear irradiance (sky plus bounce), with the identical
/// finite/range/floor rule, mask gate and `None` signals as the byte path.
pub fn sample_lightmap_hdr(bake: &ArenaBake, u: f32, v: f32) -> Option<[f32; 3]> {
    if bake.map.w == 0 || bake.map.h == 0 {
        return None;
    }
    if !(u.is_finite() && v.is_finite() && (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)) {
        return None;
    }
    // The byte path's floor rule through the shared named conversions, so both
    // lanes land on the same texel without a second index rule.
    let (w, h) = (bake.map.w, bake.map.h);
    let x = u32_from_f32((u * f32_from_u32(w)).floor()).min(w - 1);
    let y = u32_from_f32((v * f32_from_u32(h)).floor()).min(h - 1);
    let i = usize::try_from(u64::from(y) * u64::from(w) + u64::from(x)).ok()?;
    if bake.valid.get(i).copied().unwrap_or(0) != 1 {
        return None;
    }
    bake.texel_hdr(x, y)
}

/// Nearest probe coefficients in grid order (clamped indices, never panics).
fn nearest_coeffs(bake: &ArenaBake, pos: [f32; 3]) -> Option<[[f32; 3]; 9]> {
    let (nx, ny) = (bake.probes.nx as usize, bake.probes.ny as usize);
    let nz = bake.probes.nz as usize;
    if bake.probes.sh.is_empty() || nx == 0 || ny == 0 || nz == 0 {
        return None;
    }
    let axis = |p: f32, min: f32, step: f32, n: usize| {
        ((p - min) / step.max(1e-9))
            .round()
            .clamp(0.0, (n - 1) as f32) as usize
    };
    let x = axis(pos[0], bake.probes.min[0], bake.probes.step[0], nx);
    let y = axis(pos[1], bake.probes.min[1], bake.probes.step[1], ny);
    let z = axis(pos[2], bake.probes.min[2], bake.probes.step[2], nz);
    bake.probes.sh.get((z * ny + y) * nx + x).copied()
}

/// Fighter diffuse irradiance at `pos` for `normal`, evaluated through the
/// shared `ibl` SH path: trilinear coefficients outside sealed boxes, the
/// nearest sealed probe inside the gate tunnel (no cross-wall bleed).
/// Presentation only; allocates nothing per call.
pub fn fighter_irradiance(
    bake: &ArenaBake,
    pos: [f32; 3],
    normal: [f32; 3],
) -> Result<[f32; 3], BakedGiError> {
    if !pos.iter().all(|c| c.is_finite()) {
        return Err(BakedGiError::Invalid(format!(
            "baked-gi fighter pos {pos:?} is not finite"
        )));
    }
    if !normal.iter().all(|c| c.is_finite()) {
        return Err(BakedGiError::Invalid(format!(
            "baked-gi fighter normal {normal:?} is not finite"
        )));
    }
    let l2 = normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2];
    if !(l2.is_finite() && l2 >= 1e-12) {
        return Err(BakedGiError::Invalid(format!(
            "baked-gi fighter normal {normal:?} has ~zero length"
        )));
    }
    let coeffs = if bake.boxes.iter().any(|b| b.contains(pos)) {
        nearest_coeffs(bake, pos)
            .ok_or_else(|| BakedGiError::Invalid("baked-gi probe grid is empty".into()))?
    } else {
        bake.probes.sample_trilerp(pos)
    };
    // Single SH owner: the coefficients ride `ibl` eval, the same dot the
    // sky/IBL ambient takes, so probes and IBL can never drift apart.
    Ok(eval_sh(&IrradianceSh { coeffs }, normal)?)
}

/// One presentation sample for the WIN1/ED1 arena frame: static light plus
/// fighter ambient in a single call. `light` is `None` on invalid texels
/// (shade from `irradiance` there); `sealed` names the tunnel path taken.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GiFrameSample {
    /// Byte-exact palette row, or `None` on an invalid texel.
    pub light: Option<[u8; 3]>,
    /// Linear HDR companion of `light` at the same texel: the RGB9E5 lane
    /// decoded (sky plus bounce), `None` where `light` is `None`. The lane the
    /// forward swap reads; the byte lane above keeps today's look.
    pub light_hdr: Option<[f32; 3]>,
    /// Fighter diffuse irradiance in linear space, always finite.
    pub irradiance: [f32; 3],
    /// True while the position sat inside a sealed box (nearest probe).
    pub sealed: bool,
}

/// Sample one arena frame point: lightmap texel at `(u, v)` plus fighter
/// irradiance at `pos` for `normal`. The call WIN1 makes per static fragment
/// and per fighter each frame; steady-state frames allocate nothing.
pub fn sample_arena_frame(
    bake: &ArenaBake,
    u: f32,
    v: f32,
    pos: [f32; 3],
    normal: [f32; 3],
) -> Result<GiFrameSample, BakedGiError> {
    let light = sample_lightmap(bake, u, v);
    let light_hdr = sample_lightmap_hdr(bake, u, v);
    let irradiance = fighter_irradiance(bake, pos, normal)?;
    Ok(GiFrameSample {
        light,
        light_hdr,
        irradiance,
        sealed: bake.boxes.iter().any(|b| b.contains(pos)),
    })
}

/// Time one 8-fighter frame of probe lookups in ms (evidence only, never
/// hashed): the slice of [`GI_FRAME_BUDGET_MS`] this module owns.
pub fn bench_fighter_frame(
    bake: &ArenaBake,
    positions: &[[[f32; 3]; 8]],
) -> Result<f64, BakedGiError> {
    let started = Instant::now();
    for row in positions {
        for pos in row {
            let _ = fighter_irradiance(bake, *pos, [0.0, 1.0, 0.0])?;
        }
    }
    Ok(started.elapsed().as_secs_f64() * 1000.0)
}

// ---------------------------------------------------------------------------
// Frame bind: validated receipt for the forward pass, 0 validation errors.
// ---------------------------------------------------------------------------

/// Validate the per-frame bind args with the landed look rules: nonzero
/// viewport inside the frame bound, finite exposure anchor, finite nonzero
/// sun direction. Plain args, never a mirrored `LookFrame`.
pub fn validate_frame_args(
    viewport_px: [u32; 2],
    exposure_ev100: f32,
    sun_dir: [f32; 3],
) -> Result<(), BakedGiError> {
    if viewport_px[0] == 0 || viewport_px[1] == 0 {
        return Err(BakedGiError::Invalid(format!(
            "baked-gi viewport {}x{} is zero",
            viewport_px[0], viewport_px[1]
        )));
    }
    if viewport_px[0] > MAX_VIEWPORT_PX || viewport_px[1] > MAX_VIEWPORT_PX {
        return Err(BakedGiError::Invalid(format!(
            "baked-gi viewport {}x{} exceeds {MAX_VIEWPORT_PX}",
            viewport_px[0], viewport_px[1]
        )));
    }
    if !exposure_ev100.is_finite() {
        return Err(BakedGiError::Invalid(
            "baked-gi exposure_ev100 is not finite".into(),
        ));
    }
    if !sun_dir.iter().all(|c| c.is_finite()) {
        return Err(BakedGiError::Invalid(
            "baked-gi sun_dir is not finite".into(),
        ));
    }
    let len2 = sun_dir[0] * sun_dir[0] + sun_dir[1] * sun_dir[1] + sun_dir[2] * sun_dir[2];
    if !(len2.is_finite() && len2 >= 1e-12) {
        return Err(BakedGiError::Invalid(
            "baked-gi sun_dir has ~zero length".into(),
        ));
    }
    Ok(())
}

/// One bound GI frame receipt: what the forward pass reads, how big, how
/// long the bake took, with zero validation errors on the green path.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GiReceipt {
    /// Frame counter the bind carried.
    pub frame_index: u64,
    /// Viewport in pixels the bind carried.
    pub viewport_px: [u32; 2],
    /// Lightmap texels bound.
    pub lightmap_texels: u64,
    /// Valid lightmap texels (mask reads 1).
    pub valid_texels: u64,
    /// Distinct decoded HDR levels the RGB9E5 lane carries (the byte lane
    /// shows at most [`crate::gi::PALETTE_LEN`] of them).
    pub lightmap_hdr_levels: u64,
    /// Fighter probes bound.
    pub probe_count: u64,
    /// Sealed probes bound.
    pub sealed_probes: u64,
    /// Exact resident bytes (must equal [`ArenaBake::resident_bytes`]).
    pub resident_bytes: u64,
    /// wgpu validation errors observed; green means 0 (no device calls on
    /// this std-only path, so binds stay clean by construction).
    pub validation_errors: u32,
    /// Wall-clock bake time in ms (evidence only, never hashed).
    pub ms: f64,
}

impl GiReceipt {
    /// Posted frame budget this bind is charged against.
    pub fn budget_ms(&self) -> f64 {
        GI_FRAME_BUDGET_MS
    }

    /// True while the bake stayed inside the posted frame budget.
    pub fn within_budget(&self) -> bool {
        self.ms <= GI_FRAME_BUDGET_MS
    }

    /// True when the bind observed zero validation errors.
    pub fn is_clean(&self) -> bool {
        self.validation_errors == 0
    }

    /// Fraction of lightmap texels the consumer may read directly.
    pub fn valid_fraction(&self) -> f64 {
        if self.lightmap_texels == 0 {
            return 0.0;
        }
        self.valid_texels as f64 / self.lightmap_texels as f64
    }
}

/// Bind the bake for one frame: validates the frame args and the bake
/// backing, then returns the receipt the RA2 graph files under the GI pass.
/// Real validation work, never silent success; the GPU texture bind lands
/// with the RA1 bind group (orchestrator-side) reading [`ArenaBake::to_bytes`].
pub fn bind_gi(
    frame_index: u64,
    viewport_px: [u32; 2],
    exposure_ev100: f32,
    sun_dir: [f32; 3],
    bake: &ArenaBake,
    bake_ms: f64,
) -> Result<GiReceipt, BakedGiError> {
    validate_frame_args(viewport_px, exposure_ev100, sun_dir)?;
    bake.check_backing("bind")?;
    if !bake_ms.is_finite() || bake_ms < 0.0 {
        return Err(BakedGiError::Invalid(format!(
            "baked-gi bake ms {bake_ms} is not finite nonnegative"
        )));
    }
    Ok(GiReceipt {
        frame_index,
        viewport_px,
        lightmap_texels: bake.texels() as u64,
        valid_texels: bake.valid_count() as u64,
        lightmap_hdr_levels: bake.hdr_levels,
        probe_count: bake.probe_count() as u64,
        sealed_probes: bake.sealed_count() as u64,
        resident_bytes: bake.resident_bytes(),
        validation_errors: 0,
        ms: bake_ms,
    })
}

// ---------------------------------------------------------------------------
// Upload packing: verbatim cook bytes for the device bind, headless.
// ---------------------------------------------------------------------------

/// Upload texel width in bytes: RGBA8 holding the palette RGB plus the
/// validity mask in alpha (255 sound, 0 quarantined), so one texture binds
/// light and mask together and the forward pass never reads a poisoned texel.
pub const LIGHTMAP_UPLOAD_BYTES_PER_TEXEL: usize = 4;
/// Probe uniform stride in bytes: 9 RGB coefficients as vec4f32 (RGB in xyz,
/// w pads) in grid order, so the GPU dot matches [`eval_sh`] term for term.
pub const PROBE_UNIFORM_BYTES_PER_PROBE: usize = 144;
/// Graph resources the forward pass reads from the baked GI: lightmap
/// texture, probe SH storage and seal flags. Owned here so the graph names
/// the same strings the bind carries; [`compile_baked_gi_for_forward`] wires
/// them.
pub const FORWARD_GI_INPUTS: [&str; 3] = ["gi_lightmap", "gi_probes", "gi_seals"];
/// Bake pass name producing [`FORWARD_GI_INPUTS`].
pub const GI_BAKE_PASS: &str = "gi-bake";
/// Forward pass name consuming [`FORWARD_GI_INPUTS`] (its own mini-graph;
/// the arena forward names its own pass the same way).
pub const GI_FORWARD_PASS: &str = "forward";
/// Posted encode + submit + readback budget for the GI bind probe in ms
/// (debug build on the reference machine, evidence only, never hashed).
pub const GI_SUBMIT_BUDGET_MS: f64 = 16.0;
/// Posted CPU/GPU bind parity in absolute f32 units: the bind probe copies
/// cook bytes verbatim, so only exact equality passes (a flipped axis or a
/// dropped seal misses by far more).
pub const GI_BIND_PARITY_TOL: f32 = 0.0;

/// Exact RGBA8 upload length in bytes for this bake (`w * h * 4`).
/// Refuses ragged backings before any bind trusts the length.
pub fn lightmap_rgba_len(bake: &ArenaBake) -> Result<usize, BakedGiError> {
    bake.check_backing("upload")?;
    let w = usize::try_from(bake.map.w)
        .map_err(|_| BakedGiError::Invalid("baked-gi lightmap width exceeds usize".into()))?;
    let h = usize::try_from(bake.map.h)
        .map_err(|_| BakedGiError::Invalid("baked-gi lightmap height exceeds usize".into()))?;
    w.checked_mul(h)
        .and_then(|n| n.checked_mul(LIGHTMAP_UPLOAD_BYTES_PER_TEXEL))
        .ok_or_else(|| BakedGiError::Invalid("baked-gi lightmap upload length overflows".into()))
}

/// Pack the lightmap for upload into caller-owned storage: byte-exact
/// palette rows with the mask in alpha. Slices in, slices out, so
/// steady-state repacks grow nothing; the same bake gives the same bytes on
/// every run.
pub fn lightmap_rgba_bytes_into(bake: &ArenaBake, out: &mut [u8]) -> Result<(), BakedGiError> {
    if out.len() != lightmap_rgba_len(bake)? {
        return Err(BakedGiError::Invalid(format!(
            "baked-gi lightmap scratch {} mismatches upload length",
            out.len()
        )));
    }
    for (i, idx) in bake.map.idx.iter().enumerate() {
        let rgb = palette_entry(*idx);
        let base = i * LIGHTMAP_UPLOAD_BYTES_PER_TEXEL;
        out[base..base + 3].copy_from_slice(&rgb);
        out[base + 3] = if bake.valid[i] == 1 { 255 } else { 0 };
    }
    Ok(())
}

/// Exact probe uniform length in bytes (`probe_count * 144`).
pub fn probe_uniform_len(bake: &ArenaBake) -> Result<usize, BakedGiError> {
    bake.check_backing("upload")?;
    bake.probe_count()
        .checked_mul(PROBE_UNIFORM_BYTES_PER_PROBE)
        .ok_or_else(|| BakedGiError::Invalid("baked-gi probe upload length overflows".into()))
}

/// Pack the probe SH grid for upload into caller-owned storage: 9 vec4f32
/// per probe in grid order. The window submitter reuses one scratch buffer
/// across frames, so steady-state binds grow nothing.
pub fn probe_uniform_bytes_into(bake: &ArenaBake, out: &mut [u8]) -> Result<(), BakedGiError> {
    if out.len() != probe_uniform_len(bake)? {
        return Err(BakedGiError::Invalid(format!(
            "baked-gi probe scratch {} mismatches upload length",
            out.len()
        )));
    }
    let pad = 0.0f32.to_le_bytes();
    for (p, coeffs) in bake.probes.sh.iter().enumerate() {
        for (c, rgb) in coeffs.iter().enumerate() {
            let base = (p * 9 + c) * 16;
            out[base..base + 4].copy_from_slice(&rgb[0].to_le_bytes());
            out[base + 4..base + 8].copy_from_slice(&rgb[1].to_le_bytes());
            out[base + 8..base + 12].copy_from_slice(&rgb[2].to_le_bytes());
            out[base + 12..base + 16].copy_from_slice(&pad);
        }
    }
    Ok(())
}

/// Exact seal length in bytes: one flag per probe in grid order.
pub fn probe_seal_len(bake: &ArenaBake) -> Result<usize, BakedGiError> {
    bake.check_backing("upload")?;
    Ok(bake.sealed.len())
}

/// Pack the per-probe tunnel seals into caller-owned storage (1 where the
/// probe sits sealed in the tunnel, 0 where the frame blends). Same order as
/// the uniform above, so the bind reads both with one index.
pub fn probe_seal_bytes_into(bake: &ArenaBake, out: &mut [u8]) -> Result<(), BakedGiError> {
    if out.len() != probe_seal_len(bake)? {
        return Err(BakedGiError::Invalid(format!(
            "baked-gi seal scratch {} mismatches probe count",
            out.len()
        )));
    }
    out.copy_from_slice(&bake.sealed);
    Ok(())
}

// ---------------------------------------------------------------------------
// Forward GI bind (headless): the sampling chunk plus the wiring the
// forward pass binds. These carry no wgpu types, so they live outside
// `mod gpu`: the headless bind lock below hashes and packs them with no
// device, and `mod gpu` re-exports the chunk for the live submit path. New
// module edge: `graph` reads `FORWARD_GI_INPUTS` for the bake -> forward
// wiring.
// ---------------------------------------------------------------------------

/// Forward-pass GI sampling chunk (WGSL): lightmap texel with the mask in
/// alpha plus probe SH irradiance from device storage. Engine-owned code
/// (no port markers); the forward pipeline inlines this chunk when the GI
/// bind group lands. The SH basis matches [`crate::gi::sh_basis`] term for
/// term (same literals as the IBL chunk); the CPU side stays on [`eval_sh`],
/// so probes and the forward pass can never drift apart. Invalid texels
/// (alpha 0) fall back to probe ambient, the same rule as
/// [`sample_lightmap`]; the seal picks the probe index on the CPU
/// ([`fighter_irradiance`]).
pub const FORWARD_GI_WGSL: &str = r#"
@group(1) @binding(0) var gi_lightmap: texture_2d<f32>;
@group(1) @binding(1) var gi_sampler: sampler;
@group(1) @binding(2) var<storage, read> gi_probes: array<vec4<f32>>;
@group(1) @binding(3) var<storage, read> gi_seals: array<u32>;
fn gi_sh_basis(d: vec3<f32>) -> array<f32, 9> {
    var b: array<f32, 9>;
    b[0] = 0.282095;
    b[1] = -0.488603 * d.y;
    b[2] = 0.488603 * d.z;
    b[3] = -0.488603 * d.x;
    b[4] = 1.092548 * d.x * d.y;
    b[5] = -1.092548 * d.y * d.z;
    b[6] = 0.315392 * (3.0 * d.z * d.z - 1.0);
    b[7] = -1.092548 * d.x * d.z;
    b[8] = 0.546274 * (d.x * d.x - d.y * d.y);
    return b;
}
fn gi_irradiance(probe: u32, n: vec3<f32>) -> vec3<f32> {
    let b = gi_sh_basis(n);
    let base = probe * 9u;
    var e = vec3<f32>(0.0, 0.0, 0.0);
    for (var i = 0u; i < 9u; i++) {
        e += gi_probes[base + i].xyz * b[i];
    }
    return max(e, vec3<f32>(0.0, 0.0, 0.0));
}
fn gi_sample(u: f32, v: f32, probe: u32, n: vec3<f32>) -> vec3<f32> {
    let tex = textureSampleLevel(gi_lightmap, gi_sampler, vec2<f32>(u, v), 0.0);
    let ambient = gi_irradiance(probe, n);
    return tex.rgb * tex.a + ambient * (1.0 - tex.a);
}
"#;

/// Markers a compiled forward WGSL must contain to sample the baked GI: the
/// lightmap, the probe storage, the sampler, the SH basis and the sampling
/// entry. The window-only chunk above carries all five; the forward pipeline
/// inlines that chunk when the GI bind group lands.
pub const FORWARD_GI_MARKERS: [&str; 5] = [
    "gi_lightmap",
    "gi_probes",
    "gi_sampler",
    "gi_sh",
    "gi_sample",
];

/// Assert a compiled forward WGSL binds the GI chunk.
///
/// Owner: render lane; consumer: the forward presenter pipeline plus the
/// wiring gate. Units: none (string markers). Threading: pure with no IO,
/// locks or clock; safe on any thread. Determinism: the same string gives
/// the same verdict on every run. Errors: [`BakedGiError::Invalid`] naming
/// the first missing marker; empty shaders refuse instead of passing
/// silently.
///
/// ```
/// use engine_render::baked_gi::{FORWARD_GI_MARKERS, assert_forward_wgsl_binds_gi};
/// let stitched = FORWARD_GI_MARKERS.join("\n");
/// assert!(assert_forward_wgsl_binds_gi(&stitched).is_ok());
/// ```
pub fn assert_forward_wgsl_binds_gi(forward_wgsl: &str) -> Result<(), BakedGiError> {
    for marker in FORWARD_GI_MARKERS {
        if !forward_wgsl.contains(marker) {
            return Err(BakedGiError::Invalid(format!(
                "forward pass misses GI bind marker '{marker}': inline FORWARD_GI_WGSL"
            )));
        }
    }
    Ok(())
}

/// Graph hash of the bound forward chunk as a single node, pinned at cook
/// review time. The bind lock asserts this hash, so the bound chunk is
/// exactly the reviewed bytes: any drift fails loudly instead of rebinding
/// silently. Hashed through [`crate::graph::Graph::graph_hash`] (the
/// crate's hash owner) so the algorithm is owned once.
pub const FORWARD_GI_CHUNK_HASH: u64 = 1628334502214829274;

/// Hash of the bound forward chunk through the crate's graph owner; equals
/// [`FORWARD_GI_CHUNK_HASH`]. Test-only (builds one graph node);
/// per-submit packing copies the pinned const instead, so steady-state
/// submits allocate nothing.
#[cfg(test)]
fn forward_gi_chunk_probe() -> u64 {
    let mut probe = crate::graph::Graph::new();
    probe.add_node(FORWARD_GI_WGSL);
    probe.graph_hash()
}

/// Bake -> forward wiring for the GI bind (headless): producer
/// [`GI_BAKE_PASS`] outputs [`FORWARD_GI_INPUTS`], consumer
/// [`GI_FORWARD_PASS`] reads them, and compile infers the handover edges,
/// so the compiled graph carries one Transition barrier per resource from
/// bake to forward. Mirrors the IBL wiring: planning-time only (the plan
/// allocates once); per-submit packing stays on the upload packers above
/// with caller-owned storage. Fails loudly with
/// [`crate::graph::GraphError`] when the wiring stops compiling.
pub fn compile_baked_gi_for_forward(
) -> Result<crate::graph::CompiledGraph, crate::graph::GraphError> {
    let mut graph = crate::graph::FrameGraph::new();
    let bake = graph.add_pass(
        crate::graph::PassDesc::new(GI_BAKE_PASS)
            .output(FORWARD_GI_INPUTS[0])
            .output(FORWARD_GI_INPUTS[1])
            .output(FORWARD_GI_INPUTS[2])
            .work(1, 4),
    );
    let forward = graph.add_pass(
        crate::graph::PassDesc::new(GI_FORWARD_PASS)
            .input(FORWARD_GI_INPUTS[0])
            .input(FORWARD_GI_INPUTS[1])
            .input(FORWARD_GI_INPUTS[2])
            .work(2, 6),
    );
    graph.add_edge(bake, forward)?;
    graph.compile()
}

// ---------------------------------------------------------------------------
// Headless submit seam (no device in this build).
// ---------------------------------------------------------------------------

/// Validate one headless bind request: frame args, bake backing and the
/// recorded bake ms. Cold path only; the name stays off the hot list so its
/// refusal strings never count as per-frame allocation.
#[cfg(not(feature = "window"))]
fn check_frame_backing(
    viewport_px: [u32; 2],
    exposure_ev100: f32,
    sun_dir: [f32; 3],
    bake: &ArenaBake,
    bake_ms: f64,
) -> Result<(), BakedGiError> {
    validate_frame_args(viewport_px, exposure_ev100, sun_dir)?;
    bake.check_backing("submit")?;
    if !bake_ms.is_finite() || bake_ms < 0.0 {
        return Err(BakedGiError::Invalid(format!(
            "baked-gi bake ms {bake_ms} is not finite nonnegative"
        )));
    }
    Ok(())
}

/// GPU submit seam for the GI bind. Validates the frame args and the bake
/// backing like [`bind_gi`], then names the missing window device bind on
/// the headless twin (never silent success). The CPU bake above is the
/// reference here; the `window` build below records the live bind instead.
#[cfg(not(feature = "window"))]
pub fn submit_gpu(
    viewport_px: [u32; 2],
    exposure_ev100: f32,
    sun_dir: [f32; 3],
    bake: &ArenaBake,
    bake_ms: f64,
) -> Result<GiReceipt, BakedGiError> {
    check_frame_backing(viewport_px, exposure_ev100, sun_dir, bake, bake_ms)?;
    Err(BakedGiError::Invalid(
        "baked-gi gpu bind needs the window build with a live device; CPU bake is the reference"
            .into(),
    ))
}

// ---------------------------------------------------------------------------
// GPU descriptors behind `window`: real texture and storage allocation for
// the bake. Device tests skip without an adapter, exactly like the IBL and
// presenter tests. The forward chunk plus the upload packers live above (no
// wgpu types) and are re-exported here for the live submit path.
// ---------------------------------------------------------------------------

/// GI device bind for the forward presenter (window only): the lightmap
/// texture plus the probe SH storage the [`FORWARD_GI_WGSL`] chunk samples.
/// Cook bytes upload verbatim; the round-trip tests pin byte parity.
#[cfg(feature = "window")]
pub mod gpu {
    use super::{ArenaBake, BakedGiError};

    pub use super::FORWARD_GI_WGSL;

    /// Lightmap texture descriptor: `Rgba8Unorm` (palette RGB plus the mask
    /// in alpha), sampler-readable for the forward pass plus copy-dest for
    /// the cook upload and copy-src for readback.
    pub fn lightmap_desc(w: u32, h: u32) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("rl5-gi-lightmap"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }
    }

    /// Upload the lightmap cook bytes verbatim into a GPU texture. Real GPU
    /// work for the presenter GI bind group; callers keep last-good on error.
    pub fn upload_lightmap(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bake: &ArenaBake,
    ) -> Result<wgpu::Texture, BakedGiError> {
        let w = bake.map.w;
        let h = bake.map.h;
        let mut bytes = vec![0u8; super::lightmap_rgba_len(bake)?];
        if bytes.is_empty() {
            return Err(BakedGiError::Invalid(
                "baked-gi lightmap upload is empty".into(),
            ));
        }
        super::lightmap_rgba_bytes_into(bake, &mut bytes)?;
        let texel = u32::try_from(super::LIGHTMAP_UPLOAD_BYTES_PER_TEXEL).map_err(|_| {
            BakedGiError::Invalid("baked-gi lightmap texel width exceeds u32".into())
        })?;
        let stride = w.checked_mul(texel).ok_or_else(|| {
            BakedGiError::Invalid("baked-gi lightmap row stride overflows".into())
        })?;
        let texture = device.create_texture(&lightmap_desc(w, h));
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        Ok(texture)
    }

    /// Bind-probe compute shader over the probe uniform: copies the bound
    /// words back out, proving the GI bind-group path validates with 0
    /// errors and the device image matches the cook bit-for-bit.
    /// Engine-owned code (no port markers).
    const GI_BIND_WGSL: &str = r#"
@group(0) @binding(0) var<storage, read> src: array<f32>;
@group(0) @binding(1) var<storage, read_write> dst: array<f32>;
@compute @workgroup_size(64)
fn bind_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&src)) { return; }
    dst[i] = src[i];
}
"#;

    /// One window bind over the baked probes: probes bound, the worst
    /// CPU/GPU absolute difference, the validation outcome plus wall-clock
    /// ms. The live 8-fighter look proof (pilot capture) reads this receipt.
    #[derive(Clone, Copy, PartialEq, Debug)]
    pub struct GiGpuReceipt {
        /// Fighter probes bound.
        pub probe_count: u64,
        /// Worst absolute CPU/GPU difference over the uniform words.
        pub max_abs_diff: f32,
        /// wgpu validation errors observed; green means 0.
        pub validation_errors: u32,
        /// Wall time for encode + submit + readback in ms (evidence only).
        pub submit_ms: f64,
    }

    impl GiGpuReceipt {
        /// True when the bind observed zero validation errors.
        pub fn is_clean(&self) -> bool {
            self.validation_errors == 0
        }

        /// True while every bound word stayed within bind tolerance.
        pub fn within_parity(&self) -> bool {
            self.max_abs_diff <= super::GI_BIND_PARITY_TOL
        }

        /// Posted bind budget this receipt is charged against.
        pub fn budget_ms(&self) -> f64 {
            super::GI_SUBMIT_BUDGET_MS
        }
    }

    /// Reusable window submit state: the bind-probe pipeline plus one
    /// storage pair and one readback sized on the first submit, with one
    /// bind group and one scratch uniform repacked per bake. Built once per
    /// frame loop; steady-state submits create no buffers or bind groups
    /// (the reuse test proves the count stays flat).
    pub struct GiSubmitter {
        pipeline: wgpu::ComputePipeline,
        layout: wgpu::BindGroupLayout,
        src_buf: Option<wgpu::Buffer>,
        dst_buf: Option<wgpu::Buffer>,
        readback: Option<wgpu::Buffer>,
        bind: Option<wgpu::BindGroup>,
        scratch: Vec<u8>,
        sized_words: u64,
        created_buffers: u64,
    }

    impl GiSubmitter {
        /// Compile the bind-probe pipeline and layout. Storage sizes on the
        /// first submit (the bake shape is fixed per arena, so exactly once
        /// per shape; a new shape resizes once and stays flat after).
        pub fn new(device: &wgpu::Device) -> Self {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("rl5-gi-bind"),
                source: wgpu::ShaderSource::Wgsl(GI_BIND_WGSL.into()),
            });
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("rl5-gi-bind-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
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
                label: Some("rl5-gi-pipe-layout"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("rl5-gi-pipeline"),
                layout: Some(&pipe_layout),
                module: &shader,
                entry_point: Some("bind_main"),
                compilation_options: Default::default(),
                cache: None,
            });
            Self {
                pipeline,
                layout,
                src_buf: None,
                dst_buf: None,
                readback: None,
                bind: None,
                scratch: Vec::new(),
                sized_words: 0,
                created_buffers: 0,
            }
        }

        /// Sized uniform words bound so far (probe for the reuse test).
        pub fn sized_words(&self) -> u64 {
            self.sized_words
        }

        /// Sized buffers created so far (probe for the reuse test: flat
        /// across submits once the bake shape is sized).
        pub fn created_buffers(&self) -> u64 {
            self.created_buffers
        }

        /// Bind one bake: repack the probe uniform into the reused scratch,
        /// run the bind probe and score the device image against the cook.
        /// Fails loudly (never silent, never the twin refusal) on bad bakes
        /// or any device fault. Fallible text (word counts, validation,
        /// scoring) lives in cold helpers below, so this hot path keeps the
        /// crate's no-steady-state-allocation budget.
        pub fn submit(
            &mut self,
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            bake: &ArenaBake,
        ) -> Result<GiGpuReceipt, BakedGiError> {
            use std::time::Instant;
            let started = Instant::now();
            let byte_len = super::probe_uniform_len(bake)?;
            let (words_usize, words, count) = probe_word_count(byte_len)?;
            self.scratch.resize(byte_len, 0);
            super::probe_uniform_bytes_into(bake, &mut self.scratch)?;
            self.ensure_buffers(device, words, byte_len)?;
            let (src_buf, dst_buf, readback, bind) =
                match (&self.src_buf, &self.dst_buf, &self.readback, &self.bind) {
                    (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
                    _ => {
                        return Err(BakedGiError::Invalid(
                            "baked-gi submitter lost its sized buffers".into(),
                        ));
                    }
                };
            queue.write_buffer(src_buf, 0, &self.scratch);
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rl5-gi-frame"),
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("rl5-gi-bind-pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, bind, &[]);
                pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
            }
            let size = u64::try_from(byte_len).map_err(|_| {
                BakedGiError::Invalid("baked-gi probe bind size exceeds u64".into())
            })?;
            encoder.copy_buffer_to_buffer(dst_buf, 0, readback, 0, size);
            queue.submit(Some(encoder.finish()));
            gi_map_and_wait(device, readback)?;
            let back = gi_readback_f32(readback, words_usize)?;
            readback.unmap();
            check_scope(pollster::block_on(scope.pop()))?;
            let max_abs = score_bind(&back, &self.scratch)?;
            let probe_count = u64::try_from(bake.probe_count())
                .map_err(|_| BakedGiError::Invalid("baked-gi probe count exceeds u64".into()))?;
            Ok(GiGpuReceipt {
                probe_count,
                max_abs_diff: max_abs,
                validation_errors: 0,
                submit_ms: started.elapsed().as_secs_f64() * 1000.0,
            })
        }

        /// Size the storage pair, readback and bind group for this word
        /// count, once per bake shape. Old buffers drop here on a shape
        /// change; steady-state submits never reach this body.
        fn ensure_buffers(
            &mut self,
            device: &wgpu::Device,
            words: u64,
            byte_len: usize,
        ) -> Result<(), BakedGiError> {
            if self.bind.is_some() && self.sized_words == words {
                return Ok(());
            }
            let size = u64::try_from(byte_len).map_err(|_| {
                BakedGiError::Invalid("baked-gi probe bind size exceeds u64".into())
            })?;
            let src_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rl5-gi-src"),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let dst_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rl5-gi-dst"),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rl5-gi-readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rl5-gi-bind"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: src_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: dst_buf.as_entire_binding(),
                    },
                ],
            });
            self.src_buf = Some(src_buf);
            self.dst_buf = Some(dst_buf);
            self.readback = Some(readback);
            self.bind = Some(bind);
            self.sized_words = words;
            self.created_buffers = self.created_buffers.saturating_add(3);
            Ok(())
        }
    }

    /// Split a uniform byte length into f32 words for dispatch: usable
    /// lengths are nonzero multiples of 4 inside u32. Cold path only; the
    /// refusal strings never count as per-frame allocation.
    fn probe_word_count(byte_len: usize) -> Result<(usize, u64, u32), BakedGiError> {
        if byte_len == 0 || !byte_len.is_multiple_of(4) {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi probe bind length {byte_len} is not usable"
            )));
        }
        let words_usize = byte_len / 4;
        let words = u64::try_from(words_usize).map_err(|_| {
            BakedGiError::Invalid(format!("baked-gi probe words {words_usize} exceed u64"))
        })?;
        let count = u32::try_from(words).map_err(|_| {
            BakedGiError::Invalid(format!("baked-gi probe words {words} exceed u32"))
        })?;
        Ok((words_usize, words, count))
    }

    /// Turn a popped validation-scope hit into a loud refusal. Cold path only.
    fn check_scope(hit: Option<wgpu::Error>) -> Result<(), BakedGiError> {
        if let Some(err) = hit {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi bind validation: {err:?}"
            )));
        }
        Ok(())
    }

    /// Score one bind-probe readback against the packed cook: worst absolute
    /// difference, refusing loudly past parity. Cold path only.
    fn score_bind(back: &[f32], scratch: &[u8]) -> Result<f32, BakedGiError> {
        let mut max_abs: f32 = 0.0;
        for (i, got) in back.iter().enumerate() {
            let base = i * 4;
            let want = f32::from_le_bytes([
                scratch[base],
                scratch[base + 1],
                scratch[base + 2],
                scratch[base + 3],
            ]);
            let diff = (*got - want).abs();
            if diff > max_abs {
                max_abs = diff;
            }
            if !got.is_finite() || diff > super::GI_BIND_PARITY_TOL {
                return Err(BakedGiError::Invalid(format!(
                    "baked-gi bind word {i} differs: {got} vs {want}"
                )));
            }
        }
        Ok(max_abs)
    }

    /// Map a readback buffer and wait for it. One poll covers every pending map.
    fn gi_map_and_wait(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Result<(), BakedGiError> {
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| BakedGiError::Invalid(format!("baked-gi device poll: {e}")))?;
        rx.recv()
            .map_err(|e| BakedGiError::Invalid(format!("baked-gi map channel: {e}")))?
            .map_err(|e| BakedGiError::Invalid(format!("baked-gi map: {e}")))?;
        Ok(())
    }

    /// Read `count` little-endian f32 values from a mapped readback buffer.
    fn gi_readback_f32(buffer: &wgpu::Buffer, count: usize) -> Result<Vec<f32>, BakedGiError> {
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|e| BakedGiError::Invalid(format!("baked-gi mapped range: {e}")))?;
        if mapped.len() < count * 4 {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi readback {} bytes holds fewer than {count} f32 values",
                mapped.len()
            )));
        }
        let mut out = Vec::with_capacity(count);
        for chunk in mapped[..count * 4].chunks_exact(4) {
            out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        Ok(out)
    }
}

// Canonical arena fixture lives in `baked_bake` (single owner);
// re-exported above for the existing `baked_gi` paths.

// (Fixture helpers live in `baked_bake`; see re-exports above.)

// ---------------------------------------------------------------------------
// Tests: identical bytes x2, mask, shared SH, tunnel seal, loud failures.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::baked_bake::sample_sound;
    use crate::gi::{
        decode_rgb9e5, encode_rgb9e5, hdr_texel_levels, BakeScene, DirLight, Lightmap, PointLight,
        ProbeGrid, SkyAmbient, SurfaceSample, PALETTE_LEN,
    };

    fn small_samples(w: u32, h: u32) -> Vec<SurfaceSample> {
        arena_floor_samples(w, h)
    }

    fn small_scene() -> (Vec<DirLight>, Vec<PointLight>, SkyAmbient) {
        arena_fixture_parts()
    }

    fn bake_small(w: u32, h: u32) -> (ArenaBake, f64) {
        let (dirs, points, sky) = small_scene();
        let scene = BakeScene {
            dirs: &dirs,
            points: &points,
            sky,
        };
        bake_arena(
            w,
            h,
            &small_samples(w, h),
            &scene,
            &ProbeSpec {
                nx: 2,
                ny: 2,
                nz: 2,
                min: [-10.0, 0.25, -10.0],
                step: [20.0, 4.0, 20.0],
            },
            &[],
        )
        .expect("small bake builds")
    }

    #[test]
    fn bake_identical_bytes_twice() {
        // RL5 determinism proof: same complete inputs twice give bit-identical
        // bytes and one stable digest.
        let (a, ms_a) = bake_arena_fixture().expect("first bakes");
        let (b, ms_b) = bake_arena_fixture().expect("second bakes");
        assert!(ms_a >= 0.0 && ms_b >= 0.0, "bake records pass ms");
        assert_eq!(a.to_bytes(), b.to_bytes(), "bake bytes stable x2");
        assert_eq!(a.digest(), b.digest(), "digest stable x2");
        assert_ne!(a.digest(), 0, "digest must actually hash");
        // Header beyond resident bytes: dims 8 + grid axes 12 + origin/step
        // 24 + box count 4 = 48.
        assert_eq!(a.to_bytes().len(), a.resident_bytes() as usize + 48);
        assert_eq!(a.valid_count(), a.texels(), "fixture is fully valid");
        assert!(a.sealed_count() > 0, "tunnel must seal probes");
    }

    #[test]
    fn different_scene_moves_bytes_and_digest() {
        // Counter-test: the x2 proof above is not a constant fixture.
        let (a, _) = bake_arena_fixture().expect("fixture bakes");
        let (dirs, points, _) = small_scene();
        let dusk = BakeScene {
            dirs: &dirs,
            points: &points,
            sky: SkyAmbient {
                sky: [0.8, 0.3, 0.15],
                ground: [0.2, 0.12, 0.08],
            },
        };
        let w = ARENA_LIGHTMAP_W;
        let h = ARENA_LIGHTMAP_H;
        let (b, _) = bake_arena(
            w,
            h,
            &small_samples(w, h),
            &dusk,
            &ProbeSpec::arena(),
            &[arena_tunnel()],
        )
        .expect("dusk bakes");
        assert_ne!(a.to_bytes(), b.to_bytes(), "sky moves the bytes");
        assert_ne!(a.digest(), b.digest(), "sky moves the digest");
    }

    #[test]
    fn validity_mask_quarantines_degenerate_texels() {
        // Invalid texels bake black, read None, and never poison neighbors.
        let w = 4u32;
        let h = 4u32;
        let mut samples = small_samples(w, h);
        samples[5] = SurfaceSample {
            pos: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 0.0],
            albedo: [0.6, 0.5, 0.36],
            occlusion: 1.0,
        };
        samples[6] = SurfaceSample {
            pos: [1.0, 0.0, 1.0],
            normal: [0.0, 1.0, 0.0],
            albedo: [f32::NAN, 0.5, 0.36],
            occlusion: 1.0,
        };
        samples[9] = SurfaceSample {
            pos: [2.0, 0.0, 2.0],
            normal: [f32::INFINITY, 0.0, 0.0],
            albedo: [0.6, 0.5, 0.36],
            occlusion: 1.0,
        };
        samples[10] = SurfaceSample {
            pos: [3.0, 0.0, 3.0],
            normal: [0.0, 1.0, 0.0],
            albedo: [0.6, 0.5, 0.36],
            occlusion: f32::NAN,
        };
        let (dirs, points, sky) = small_scene();
        let scene = BakeScene {
            dirs: &dirs,
            points: &points,
            sky,
        };
        let (bake, _) = bake_arena(
            w,
            h,
            &samples,
            &scene,
            &ProbeSpec {
                nx: 2,
                ny: 1,
                nz: 2,
                min: [-10.0, 0.25, -10.0],
                step: [20.0, 4.0, 20.0],
            },
            &[],
        )
        .expect("poisoned bake builds");
        assert_eq!(bake.valid_count(), 12, "four texels quarantined");
        assert!(!bake.is_valid(1, 1), "zero normal is invalid");
        assert!(!bake.is_valid(2, 1), "NaN albedo is invalid");
        assert!(!bake.is_valid(1, 2), "infinite normal is invalid");
        assert!(!bake.is_valid(2, 2), "NaN occlusion is invalid");
        assert!(bake.is_valid(0, 0), "sound texel stays valid");
        assert!(!bake.is_valid(9, 9), "outside dims reads invalid");
        // Masked texels read None; the map itself holds finite black.
        for t in &bake.map.tex {
            assert!(t.iter().all(|c| c.is_finite() && *c >= 0.0));
        }
        let bad = sample_lightmap(&bake, 0.3, 0.3);
        assert_eq!(bad, None, "masked texel falls back to probes");
        let good = sample_lightmap(&bake, 0.1, 0.1).expect("valid texel reads");
        assert_eq!(good, palette_entry(bake.map.idx[0]));
        // The linear lane takes the same mask gate and the same floor rule.
        assert_eq!(sample_lightmap_hdr(&bake, 0.3, 0.3), None, "masked lane");
        assert_eq!(sample_lightmap_hdr(&bake, 0.1, 0.1), bake.texel_hdr(0, 0));
        // Fallback irradiance still works over masked texels.
        let fb =
            fighter_irradiance(&bake, [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]).expect("fallback shades");
        assert!(fb.iter().all(|c| c.is_finite() && *c >= 0.0));
    }

    #[test]
    fn valid_texels_match_gi_palette_rows() {
        // Lightmap reads are byte-exact `gi` palette rows, never blends.
        let (bake, _) = bake_small(8, 8);
        for y in 0..8u32 {
            for x in 0..8u32 {
                let u = (f64::from(x) + 0.5) / 8.0;
                let v = (f64::from(y) + 0.5) / 8.0;
                let got = sample_lightmap(&bake, u as f32, v as f32).expect("fixture valid");
                let i = y as usize * 8 + x as usize;
                assert_eq!(got, palette_entry(bake.map.idx[i]), "texel ({x},{y}) exact");
            }
        }
        assert_eq!(
            sample_lightmap(&bake, -0.1, 0.5),
            None,
            "left of map misses"
        );
        assert_eq!(sample_lightmap(&bake, 0.5, 1.5), None, "below map misses");
        assert_eq!(sample_lightmap(&bake, f32::NAN, 0.5), None, "NaN misses");
        let empty = ArenaBake {
            map: Lightmap {
                w: 0,
                h: 0,
                tex: vec![],
                idx: vec![],
            },
            valid: vec![],
            hdr: vec![],
            hdr_levels: 0,
            probes: ProbeGrid {
                nx: 0,
                ny: 0,
                nz: 0,
                min: [0.0; 3],
                step: [1.0; 3],
                sh: vec![],
            },
            sealed: vec![],
            boxes: vec![],
        };
        assert_eq!(sample_lightmap(&empty, 0.5, 0.5), None, "empty map misses");
    }

    #[test]
    fn probes_evaluate_through_the_shared_ibl_path() {
        // CODELAW one owner: every probe coefficient rides `ibl::eval_sh`,
        // so this test replays the same dot through `gi` and demands exact
        // agreement (both clamp at zero; the fixture sky keeps lobes finite).
        let (bake, _) = bake_arena_fixture().expect("fixture bakes");
        for probe in &bake.probes.sh {
            let via_ibl =
                eval_sh(&IrradianceSh { coeffs: *probe }, [0.0, 1.0, 0.0]).expect("probe evals");
            let mut raw = crate::gi::sh_irradiance(probe, [0.0, 1.0, 0.0]);
            for c in &mut raw {
                *c = c.max(0.0);
            }
            assert_eq!(via_ibl, raw, "shared SH dot agrees term for term");
        }
        // The fighter path agrees with the grid path on the same position.
        let pos = [4.0, 1.0, -4.0];
        let trilerp = bake.probes.sample_trilerp(pos);
        let mut raw = crate::gi::sh_irradiance(&trilerp, [0.0, 1.0, 0.0]);
        for c in &mut raw {
            *c = c.max(0.0);
        }
        assert_eq!(
            fighter_irradiance(&bake, pos, [0.0, 1.0, 0.0]).expect("fighter shades"),
            raw
        );
    }

    #[test]
    fn sealed_tunnel_takes_nearest_without_bleed() {
        // Gate-tunnel edge: a fighter deep inside reads the nearest probe
        // exactly, so sunlit sand outside cannot leak across the wall.
        let (bake, _) = bake_arena_fixture().expect("fixture bakes");
        let inside = [0.0, 1.0, 10.0];
        let normal = [0.0, 1.0, 0.0];
        assert!(
            bake.boxes.iter().any(|b| b.contains(inside)),
            "probe point must sit in the tunnel"
        );
        let sealed = fighter_irradiance(&bake, inside, normal).expect("tunnel shades");
        let near = nearest_coeffs(&bake, inside).expect("nearest reads");
        let mut raw = crate::gi::sh_irradiance(&near, normal);
        for c in &mut raw {
            *c = c.max(0.0);
        }
        assert_eq!(sealed, raw, "sealed path is exactly nearest");
        let frame = sample_arena_frame(&bake, 0.5, 0.9, inside, normal).expect("frame samples");
        assert!(frame.sealed, "frame names the tunnel path");
        assert!(frame.irradiance.iter().all(|c| c.is_finite()));
        // Outside the tunnel the frame blends and names the open path.
        let open =
            sample_arena_frame(&bake, 0.1, 0.1, [8.0, 1.0, -8.0], normal).expect("open samples");
        assert!(!open.sealed, "open floor blends");
        assert!(open.light.is_some(), "open floor reads the lightmap");
    }

    #[test]
    fn seal_flags_match_probe_positions() {
        // Every sealed probe sits in the tunnel box; no open probe does.
        let (bake, _) = bake_arena_fixture().expect("fixture bakes");
        let tunnel = arena_tunnel();
        let (nx, ny) = (bake.probes.nx as usize, bake.probes.ny as usize);
        for (i, flag) in bake.sealed.iter().enumerate() {
            let x = i % nx;
            let y = (i / nx) % ny;
            let z = i / (nx * ny);
            let p = bake.probes.probe_pos(x, y, z);
            assert_eq!(
                *flag == 1,
                tunnel.contains(p),
                "probe {i} at {p:?} seal must match the box"
            );
        }
    }

    #[test]
    fn bad_bake_inputs_fail_loudly() {
        let (dirs, points, sky) = small_scene();
        let scene = BakeScene {
            dirs: &dirs,
            points: &points,
            sky,
        };
        let spec = ProbeSpec {
            nx: 2,
            ny: 1,
            nz: 2,
            min: [0.0; 3],
            step: [1.0; 3],
        };
        let ok = small_samples(4, 4);
        assert!(matches!(
            bake_arena(0, 4, &ok, &scene, &spec, &[]),
            Err(BakedGiError::Invalid(_))
        ));
        assert!(matches!(
            bake_arena(4, 4, &ok[..15], &scene, &spec, &[]),
            Err(BakedGiError::Invalid(_))
        ));
        assert!(matches!(
            bake_arena(BAKE_MAX_DIM + 1, 4, &ok, &scene, &spec, &[]),
            Err(BakedGiError::Invalid(_))
        ));
        let bad_spec = ProbeSpec { nx: 0, ..spec };
        assert!(matches!(
            bake_arena(4, 4, &ok, &scene, &bad_spec, &[]),
            Err(BakedGiError::Invalid(_))
        ));
        let fat = ProbeSpec {
            nx: PROBE_MAX_AXIS,
            ny: PROBE_MAX_AXIS,
            nz: PROBE_MAX_AXIS,
            min: [0.0; 3],
            step: [1.0; 3],
        };
        assert!(matches!(
            bake_arena(4, 4, &ok, &scene, &fat, &[]),
            Err(BakedGiError::Invalid(_))
        ));
        let flipped = SealedBox {
            min: [1.0, 0.0, 0.0],
            max: [0.0, 1.0, 1.0],
        };
        assert!(matches!(
            bake_arena(4, 4, &ok, &scene, &spec, &[flipped]),
            Err(BakedGiError::Invalid(_))
        ));
        let nan_box = SealedBox {
            min: [f32::NAN, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        };
        assert!(matches!(
            bake_arena(4, 4, &ok, &scene, &spec, &[nan_box]),
            Err(BakedGiError::Invalid(_))
        ));
        let bad_sky = BakeScene {
            dirs: &dirs,
            points: &points,
            sky: SkyAmbient {
                sky: [f32::NAN, 0.4, 0.5],
                ground: [0.2, 0.17, 0.12],
            },
        };
        assert!(matches!(
            bake_arena(4, 4, &ok, &bad_sky, &spec, &[]),
            Err(BakedGiError::Invalid(_))
        ));
        let bad_dir = [DirLight {
            dir: [0.0, 0.0, 0.0],
            color: [1.0; 3],
        }];
        let bad_scene = BakeScene {
            dirs: &bad_dir,
            points: &points,
            sky,
        };
        assert!(matches!(
            bake_arena(4, 4, &ok, &bad_scene, &spec, &[]),
            Err(BakedGiError::Invalid(_))
        ));
        let bad_point = [PointLight {
            pos: [0.0, 1.0, 0.0],
            color: [1.0; 3],
            inv_range_sq: -1.0,
        }];
        let bad_scene = BakeScene {
            dirs: &dirs,
            points: &bad_point,
            sky,
        };
        assert!(matches!(
            bake_arena(4, 4, &ok, &bad_scene, &spec, &[]),
            Err(BakedGiError::Invalid(_))
        ));
    }

    #[test]
    fn bad_lookup_and_frame_inputs_fail_loudly() {
        let (bake, _) = bake_small(4, 4);
        assert!(matches!(
            fighter_irradiance(&bake, [f32::NAN, 1.0, 0.0], [0.0, 1.0, 0.0]),
            Err(BakedGiError::Invalid(_))
        ));
        assert!(matches!(
            fighter_irradiance(&bake, [0.0, 1.0, 0.0], [0.0, 0.0, 0.0]),
            Err(BakedGiError::Invalid(_))
        ));
        assert!(matches!(
            fighter_irradiance(&bake, [0.0, 1.0, 0.0], [f32::INFINITY, 0.0, 0.0]),
            Err(BakedGiError::Invalid(_))
        ));
        assert!(matches!(
            sample_arena_frame(&bake, 0.5, 0.5, [0.0, 1.0, 0.0], [0.0, 0.0, 0.0]),
            Err(BakedGiError::Invalid(_))
        ));
        assert!(matches!(
            validate_frame_args([0, 720], 12.0, [0.0, 1.0, 0.0]),
            Err(BakedGiError::Invalid(_))
        ));
        assert!(matches!(
            validate_frame_args([1280, 720], f32::NAN, [0.0, 1.0, 0.0]),
            Err(BakedGiError::Invalid(_))
        ));
        assert!(matches!(
            validate_frame_args([1280, 720], 12.0, [0.0, 0.0, 0.0]),
            Err(BakedGiError::Invalid(_))
        ));
        assert!(matches!(
            validate_frame_args([4096, 720], 12.0, [0.0, 1.0, 0.0]),
            Err(BakedGiError::Invalid(_))
        ));
        validate_frame_args([1920, 1080], 12.0, [0.2, 1.0, 0.1]).expect("sane frame validates");
    }

    #[test]
    fn bind_receipt_carries_bytes_and_zero_errors() {
        // The consumer bind: validated receipt, exact resident bytes, clean.
        let (bake, ms) = bake_arena_fixture().expect("fixture bakes");
        let receipt =
            bind_gi(7, [1920, 1080], 12.0, [0.3, 1.0, 0.2], &bake, ms).expect("bind validates");
        assert_eq!(receipt.frame_index, 7);
        assert_eq!(receipt.viewport_px, [1920, 1080]);
        assert_eq!(
            receipt.lightmap_texels,
            ARENA_LIGHTMAP_W as u64 * ARENA_LIGHTMAP_H as u64
        );
        assert_eq!(receipt.valid_texels, receipt.lightmap_texels);
        assert_eq!(receipt.probe_count, ProbeSpec::arena().count() as u64);
        assert_eq!(receipt.sealed_probes, bake.sealed_count() as u64);
        assert_eq!(receipt.resident_bytes, bake.resident_bytes());
        assert_eq!(receipt.validation_errors, 0);
        assert!(receipt.is_clean(), "bind stays validation-clean");
        assert_eq!(receipt.budget_ms(), GI_FRAME_BUDGET_MS);
        // Wall-clock budget lives in bind_wall_budget (--perf only): debug
        // bakes measure ~25ms on the reference laptop; the 16.6ms bar is a
        // release-mode number. Exactness asserts above stay default.
        assert!((receipt.valid_fraction() - 1.0).abs() < 1e-12);
        assert!(matches!(
            bind_gi(7, [0, 1080], 12.0, [0.3, 1.0, 0.2], &bake, ms),
            Err(BakedGiError::Invalid(_))
        ));
        assert!(matches!(
            bind_gi(7, [1920, 1080], 12.0, [0.3, 1.0, 0.2], &bake, f64::NAN),
            Err(BakedGiError::Invalid(_))
        ));
        let mut ragged = bake_arena_fixture().expect("fixture bakes").0;
        ragged.valid.pop();
        assert!(matches!(
            bind_gi(7, [1920, 1080], 12.0, [0.3, 1.0, 0.2], &ragged, ms),
            Err(BakedGiError::Invalid(_))
        ));
        ragged = bake_arena_fixture().expect("fixture bakes").0;
        ragged.valid[0] = 2;
        assert!(matches!(
            bind_gi(7, [1920, 1080], 12.0, [0.3, 1.0, 0.2], &ragged, ms),
            Err(BakedGiError::Invalid(_))
        ));
    }

    #[test]
    #[ignore = "timing: 16.6ms bind budget needs release --perf on the idle reference GPU (debug bakes measure ~25ms)"]
    fn bind_wall_budget() {
        let (bake, ms) = bake_arena_fixture().expect("fixture bakes");
        let receipt =
            bind_gi(7, [1920, 1080], 12.0, [0.3, 1.0, 0.2], &bake, ms).expect("bind validates");
        // Perf harness line: log the measured bind so --perf medians are readable.
        eprintln!(
            "bind_wall_budget: measured {:.3} ms vs {:.1} ms budget",
            receipt.ms,
            receipt.budget_ms()
        );
        assert!(receipt.is_clean(), "bind stays validation-clean");
        assert!(
            receipt.within_budget(),
            "offline bake clears 16.6 ms, measured {:.2} ms",
            receipt.ms
        );
    }

    #[test]
    fn probe_spec_and_box_validation() {
        ProbeSpec::arena().validate().expect("arena spec validates");
        assert_eq!(ProbeSpec::arena().count(), 108);
        for bad in [
            ProbeSpec {
                nx: PROBE_MAX_AXIS + 1,
                ny: 1,
                nz: 1,
                min: [0.0; 3],
                step: [1.0; 3],
            },
            ProbeSpec {
                nx: 1,
                ny: 1,
                nz: 1,
                min: [f32::INFINITY, 0.0, 0.0],
                step: [1.0; 3],
            },
            ProbeSpec {
                nx: 1,
                ny: 1,
                nz: 1,
                min: [0.0; 3],
                step: [0.0, 1.0, 1.0],
            },
        ] {
            assert!(
                matches!(bad.validate(), Err(BakedGiError::Invalid(_))),
                "bad spec {bad:?} must fail loudly"
            );
        }
        arena_tunnel().validate().expect("tunnel validates");
        assert!(arena_tunnel().contains([0.0, 1.0, 10.0]));
        assert!(!arena_tunnel().contains([0.0, 1.0, 7.9]));
        assert!(arena_tunnel().contains([-3.0, 0.0, 8.0]), "edges seal");
        assert_eq!(
            format!("{}", BakedGiError::Invalid("x".into())),
            "baked-gi input: x"
        );
    }

    #[test]
    fn fixture_helpers_stay_deterministic() {
        // The shared consumer fixture replays exactly (WIN1/ED1/Q1 pin this).
        let a = arena_floor_samples(8, 8);
        let b = arena_floor_samples(8, 8);
        assert_eq!(a.len(), 64);
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(
                (x.pos, x.normal, x.albedo, x.occlusion),
                (y.pos, y.normal, y.albedo, y.occlusion)
            );
        }
        assert!(a.iter().all(sample_sound), "fixture is fully sound");
    }

    #[test]
    #[ignore = "timing: 8-fighter lookup budget, run under --perf release"]
    fn fighter_frame_clears_sample_budget() {
        // Perf gate: one 8-fighter frame of probe lookups against the posted
        // CPU slice of the 60 fps budget; prints bake digest x2 + timing.
        let (a, _) = bake_arena_fixture().expect("first bakes");
        let (b, _) = bake_arena_fixture().expect("second bakes");
        assert_eq!(a.digest(), b.digest(), "digest stable x2");
        println!("bake digest x2: {:016x}", a.digest());
        println!(
            "bake bytes: {} resident: {} valid: {}/{} probes: {} sealed: {}",
            a.to_bytes().len(),
            a.resident_bytes(),
            a.valid_count(),
            a.texels(),
            a.probe_count(),
            a.sealed_count()
        );
        let rows: Vec<[[f32; 3]; 8]> = (0..60)
            .map(|f| {
                let t = f as f32 * 0.5;
                std::array::from_fn(|k| {
                    let k = k as f32;
                    [-8.0 + k * 2.0 + t * 0.1, 1.0, -6.0 + k * 1.5 - t * 0.05]
                })
            })
            .collect();
        let ms = bench_fighter_frame(&a, &rows).expect("bench runs");
        println!("8-fighter x60 frames lookup: {ms:.3} ms total");
        assert!(
            ms / 60.0 <= FIGHTER_SAMPLE_BUDGET_MS,
            "per-frame lookup must clear {FIGHTER_SAMPLE_BUDGET_MS} ms, took {:.3}",
            ms / 60.0
        );
    }

    #[test]
    fn rl5_8fighter_capture_receipt_x2() {
        // RL5 enabler bundle: identical bake bytes x2 plus the RA1-side bind
        // receipt Q1 files plus one 8-fighter probe frame. Exactness asserts
        // run by default; wall budgets stay in the --perf gates above.
        let (a, ms_a) = bake_arena_fixture().expect("first bakes");
        let (b, ms_b) = bake_arena_fixture().expect("second bakes");
        assert_eq!(a.to_bytes(), b.to_bytes(), "bake bytes stable x2");
        assert_eq!(a.digest(), b.digest(), "digest stable x2");
        assert!(ms_a.is_finite() && ms_a >= 0.0);
        assert!(ms_b.is_finite() && ms_b >= 0.0);
        println!("rl5 bake digest x2: {:016x}", a.digest());
        println!(
            "rl5 bake bytes: {} resident: {} valid: {}/{} probes: {} sealed: {}",
            a.to_bytes().len(),
            a.resident_bytes(),
            a.valid_count(),
            a.texels(),
            a.probe_count(),
            a.sealed_count()
        );
        let receipt =
            bind_gi(3, [1280, 720], 12.0, [0.3, 1.0, 0.2], &a, ms_a).expect("bind validates");
        assert_eq!(
            receipt.lightmap_texels,
            ARENA_LIGHTMAP_W as u64 * ARENA_LIGHTMAP_H as u64
        );
        assert_eq!(receipt.valid_texels, receipt.lightmap_texels);
        assert_eq!(receipt.probe_count, ProbeSpec::arena().count() as u64);
        assert_eq!(receipt.resident_bytes, a.resident_bytes());
        assert!(receipt.is_clean(), "bind stays validation-clean");
        assert_eq!(receipt.budget_ms(), GI_FRAME_BUDGET_MS);
        // One 8-fighter frame of probe lookups shades every fighter.
        let fighters: [[f32; 3]; 8] = std::array::from_fn(|k| {
            let k = k as f32;
            [-7.0 + k * 2.0, 1.0, -6.0 + k * 1.5]
        });
        for pos in fighters {
            let shade = fighter_irradiance(&a, pos, [0.0, 1.0, 0.0]).expect("fighter shades");
            assert!(shade.iter().all(|c| c.is_finite() && *c >= 0.0));
        }
        let ms = bench_fighter_frame(&a, &[fighters]).expect("bench runs");
        println!("rl5 8-fighter x1 frame lookup: {ms:.3} ms total");
        assert!(ms.is_finite() && ms >= 0.0);
    }

    #[test]
    fn rl5_ra1_receipt_consumes_the_bind() {
        // RA1-side consumption (CPU receipt only): the presenter helper bakes
        // the same fixture and files the same validated receipt, with no
        // orientation or blit change.
        let receipt =
            crate::present::baked_gi_receipt_for_frame(9, [1280, 720], 12.0, [0.3, 1.0, 0.2])
                .expect("ra1 receipt binds");
        assert_eq!(receipt.frame_index, 9);
        assert_eq!(receipt.viewport_px, [1280, 720]);
        assert_eq!(
            receipt.lightmap_texels,
            ARENA_LIGHTMAP_W as u64 * ARENA_LIGHTMAP_H as u64
        );
        assert_eq!(receipt.valid_texels, receipt.lightmap_texels);
        assert_eq!(receipt.probe_count, ProbeSpec::arena().count() as u64);
        assert!(receipt.is_clean(), "ra1 bind stays validation-clean");
        println!(
            "rl5 ra1 receipt: frame={} viewport={:?} probes={} sealed={} resident={} valid_frac={:.3}",
            receipt.frame_index,
            receipt.viewport_px,
            receipt.probe_count,
            receipt.sealed_probes,
            receipt.resident_bytes,
            receipt.valid_fraction()
        );
        // The same bake feeds the per-fighter sample the frame makes.
        let (bake, _) = bake_arena_fixture().expect("fixture bakes");
        let sample = crate::present::baked_gi_frame_sample(
            &bake,
            0.1,
            0.1,
            [8.0, 1.0, -8.0],
            [0.0, 1.0, 0.0],
        )
        .expect("frame samples");
        assert!(sample.light.is_some(), "open floor reads the lightmap");
        assert!(!sample.sealed);
        // Failure edges: ragged frame args fail loudly, never silent green.
        assert!(
            crate::present::baked_gi_receipt_for_frame(9, [0, 720], 12.0, [0.3, 1.0, 0.2]).is_err()
        );
        assert!(crate::present::baked_gi_receipt_for_frame(
            9,
            [1280, 720],
            f32::NAN,
            [0.3, 1.0, 0.2]
        )
        .is_err());
        assert!(
            crate::present::baked_gi_receipt_for_frame(9, [1280, 720], 12.0, [0.0, 0.0, 0.0])
                .is_err()
        );
        assert!(crate::present::baked_gi_frame_sample(
            &bake,
            0.5,
            0.5,
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0]
        )
        .is_err());
    }

    #[test]
    fn upload_packers_are_verbatim_and_stable_x2() {
        // Device bind inputs: RGBA lightmap with the mask in alpha plus the
        // probe uniform and seals, bit-identical across two packs.
        let (a, _) = bake_arena_fixture().expect("fixture bakes");
        let (b, _) = bake_arena_fixture().expect("second bakes");
        assert_eq!(
            lightmap_rgba_len(&a).expect("len"),
            ARENA_LIGHTMAP_W as usize * ARENA_LIGHTMAP_H as usize * LIGHTMAP_UPLOAD_BYTES_PER_TEXEL
        );
        let mut pa = vec![0u8; lightmap_rgba_len(&a).expect("len")];
        let mut pb = vec![0u8; pa.len()];
        lightmap_rgba_bytes_into(&a, &mut pa).expect("packs");
        lightmap_rgba_bytes_into(&b, &mut pb).expect("packs");
        assert_eq!(pa, pb, "lightmap upload stable x2");
        for (i, chunk) in pa.chunks_exact(4).enumerate() {
            assert_eq!(
                &chunk[..3],
                &palette_entry(a.map.idx[i]),
                "texel {i} keeps its palette row"
            );
            assert_eq!(chunk[3], 255, "fixture texels are all valid");
        }
        // The probe uniform decodes back to the bake coefficients bit-for-bit.
        let mut ua = vec![0u8; probe_uniform_len(&a).expect("len")];
        probe_uniform_bytes_into(&a, &mut ua).expect("packs");
        assert_eq!(ua.len(), a.probe_count() * PROBE_UNIFORM_BYTES_PER_PROBE);
        let pad = 0.0f32.to_le_bytes();
        for (p, coeffs) in a.probes.sh.iter().enumerate() {
            for (c, rgb) in coeffs.iter().enumerate() {
                let base = (p * 9 + c) * 16;
                for (k, want) in rgb.iter().enumerate() {
                    let got = f32::from_le_bytes([
                        ua[base + k * 4],
                        ua[base + k * 4 + 1],
                        ua[base + k * 4 + 2],
                        ua[base + k * 4 + 3],
                    ]);
                    assert_eq!(got, *want, "probe {p} coeff {c} channel {k} exact");
                }
                assert_eq!(&ua[base + 12..base + 16], &pad, "w pads");
            }
        }
        // Seals ride grid order next to the uniform.
        let mut sa = vec![0u8; probe_seal_len(&a).expect("len")];
        probe_seal_bytes_into(&a, &mut sa).expect("packs");
        assert_eq!(sa, a.sealed, "seals pack verbatim");
        assert_eq!(
            sa.iter().filter(|&&v| v == 1).count(),
            a.sealed_count(),
            "seal bytes name the tunnel probes"
        );
    }

    #[test]
    fn masked_texels_pack_alpha_zero_with_sound_neighbors() {
        // Counter-test: the alpha channel is the mask, not a constant fill.
        let w = 4u32;
        let h = 4u32;
        let mut samples = small_samples(w, h);
        samples[5] = SurfaceSample {
            pos: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 0.0],
            albedo: [0.6, 0.5, 0.36],
            occlusion: 1.0,
        };
        let (dirs, points, sky) = small_scene();
        let scene = BakeScene {
            dirs: &dirs,
            points: &points,
            sky,
        };
        let (bake, _) = bake_arena(
            w,
            h,
            &samples,
            &scene,
            &ProbeSpec {
                nx: 2,
                ny: 1,
                nz: 2,
                min: [-10.0, 0.25, -10.0],
                step: [20.0, 4.0, 20.0],
            },
            &[],
        )
        .expect("poisoned bake builds");
        let mut packed = vec![0u8; lightmap_rgba_len(&bake).expect("len")];
        lightmap_rgba_bytes_into(&bake, &mut packed).expect("packs");
        assert_eq!(packed[5 * 4 + 3], 0, "quarantined texel packs alpha 0");
        assert_eq!(
            &packed[5 * 4..5 * 4 + 3],
            &[0, 0, 0],
            "quarantined texel bakes black"
        );
        assert_eq!(packed[3], 255, "sound neighbor keeps alpha 255");
        assert_eq!(packed[15 * 4 + 3], 255, "far texel keeps alpha 255");
    }

    #[test]
    fn upload_packers_refuse_ragged_backings() {
        // Wrong scratch lengths and ragged bakes fail loudly, never pack
        // garbage the device would bind.
        let (bake, _) = bake_small(4, 4);
        let mut short = vec![0u8; 3];
        assert!(matches!(
            lightmap_rgba_bytes_into(&bake, &mut short),
            Err(BakedGiError::Invalid(_))
        ));
        assert!(matches!(
            probe_uniform_bytes_into(&bake, &mut short),
            Err(BakedGiError::Invalid(_))
        ));
        assert!(matches!(
            probe_seal_bytes_into(&bake, &mut short),
            Err(BakedGiError::Invalid(_))
        ));
        let mut ragged = bake_arena_fixture().expect("fixture bakes").0;
        ragged.valid.pop();
        let mut scratch = vec![0u8; 16];
        assert!(matches!(
            lightmap_rgba_bytes_into(&ragged, &mut scratch),
            Err(BakedGiError::Invalid(_))
        ));
        assert!(matches!(
            probe_uniform_len(&ragged),
            Err(BakedGiError::Invalid(_))
        ));
        assert!(matches!(
            probe_seal_len(&ragged),
            Err(BakedGiError::Invalid(_))
        ));
        let mut ragged_seal = bake_arena_fixture().expect("fixture bakes").0;
        ragged_seal.sealed.pop();
        assert!(matches!(
            probe_seal_len(&ragged_seal),
            Err(BakedGiError::Invalid(_))
        ));
        let mut bad_mask = bake_arena_fixture().expect("fixture bakes").0;
        bad_mask.valid[0] = 2;
        assert!(matches!(
            lightmap_rgba_len(&bad_mask),
            Err(BakedGiError::Invalid(_))
        ));
    }

    #[test]
    fn forward_gi_chunk_is_pinned_for_the_forward_bind() {
        // The bound chunk is exactly the reviewed bytes: the hash pins drift,
        // the shape asserts pin silent rebinding (a stub chunk would miss the
        // entries, a renumbered bind group would miss the bindings).
        assert_eq!(
            forward_gi_chunk_probe(),
            FORWARD_GI_CHUNK_HASH,
            "forward chunk drifted without updating the pin"
        );
        assert!(!FORWARD_GI_WGSL.is_empty(), "forward chunk ships");
        for binding in [
            "@group(1) @binding(0)",
            "@group(1) @binding(1)",
            "@group(1) @binding(2)",
            "@group(1) @binding(3)",
        ] {
            assert!(
                FORWARD_GI_WGSL.contains(binding),
                "forward bind carries {binding}"
            );
        }
        for entry in [
            "gi_sample",
            "gi_irradiance",
            "gi_sh_basis",
            "array<vec4<f32>>",
            "array<u32>",
            "textureSampleLevel",
        ] {
            assert!(
                FORWARD_GI_WGSL.contains(entry),
                "forward chunk keeps {entry}"
            );
        }
        // The basis literals the GPU dots are named here; the shared-SH test
        // pins their values against the CPU basis, so a flipped sign fails.
        for literal in ["0.282095", "0.488603", "1.092548", "0.315392", "0.546274"] {
            assert!(
                FORWARD_GI_WGSL.contains(literal),
                "forward chunk carries basis {literal}"
            );
        }
    }

    #[test]
    fn forward_markers_gate_the_forward_bind() {
        // The wiring gate: the chunk binds itself, and dropping any marker
        // refuses loudly instead of rebinding silently.
        assert_forward_wgsl_binds_gi(FORWARD_GI_WGSL).expect("chunk binds itself");
        for marker in FORWARD_GI_MARKERS {
            let cut = FORWARD_GI_WGSL.replace(marker, "zz");
            assert!(
                assert_forward_wgsl_binds_gi(&cut).is_err(),
                "missing {marker} refuses"
            );
        }
        assert!(
            assert_forward_wgsl_binds_gi("").is_err(),
            "empty shader refuses"
        );
    }

    #[test]
    fn baked_gi_hands_resources_to_forward() {
        // Unwired is FAIL: the compiled bake -> forward graph hands every
        // bake resource to forward with a Transition barrier, forward reads
        // all three, bake runs first, and the receipt keeps both passes'
        // draws.
        let compiled = compile_baked_gi_for_forward().expect("gi wiring compiles");
        assert!(
            compiled.order_names_match(&[GI_BAKE_PASS, GI_FORWARD_PASS]),
            "bake runs before forward"
        );
        let (bake_id, forward_id) = (compiled.order()[0], compiled.order()[1]);
        assert_eq!(
            compiled.pass(bake_id).expect("dense ids").name.as_str(),
            GI_BAKE_PASS
        );
        assert_eq!(
            compiled.pass(forward_id).expect("dense ids").name.as_str(),
            GI_FORWARD_PASS
        );
        for resource in FORWARD_GI_INPUTS {
            assert!(
                compiled.barriers().iter().any(|b| {
                    b.resource == resource
                        && b.kind == crate::graph::BarrierKind::Transition
                        && b.from == bake_id
                        && b.to == forward_id
                }),
                "{resource} hands over to forward"
            );
        }
        let forward = compiled.pass(forward_id).expect("forward reads");
        for resource in FORWARD_GI_INPUTS {
            assert!(
                forward.inputs.iter().any(|i| i == resource),
                "forward reads {resource}: {:?}",
                forward.inputs
            );
        }
        let mut compiled = compiled;
        let receipt = compiled.execute(&crate::graph::FrameInfo {
            frame_index: 7,
            width_px: 16,
            height_px: 16,
        });
        assert_eq!(receipt.draws, 3, "both passes record draws");
        assert_eq!(receipt.triangles, 10, "both passes record triangles");
        assert_eq!(receipt.per_pass_cpu_ms.len(), 2);
    }

    #[test]
    fn steady_state_frame_paths_allocate_nothing() {
        // CODELAW budget: per-frame GI lookups and repacks grow nothing.
        let (bake, _) = bake_arena_fixture().expect("fixture bakes");
        let mut rgba = vec![0u8; lightmap_rgba_len(&bake).expect("len")];
        let mut uniform = vec![0u8; probe_uniform_len(&bake).expect("len")];
        let mut seals = vec![0u8; probe_seal_len(&bake).expect("len")];
        let ((), heap) = engine_core::alloc_probe::measure(|| {
            lightmap_rgba_bytes_into(&bake, &mut rgba).expect("packs");
            probe_uniform_bytes_into(&bake, &mut uniform).expect("packs");
            probe_seal_bytes_into(&bake, &mut seals).expect("packs");
            let _ = fighter_irradiance(&bake, [4.0, 1.0, -4.0], [0.0, 1.0, 0.0]).expect("shades");
            let _ = sample_arena_frame(&bake, 0.1, 0.1, [8.0, 1.0, -8.0], [0.0, 1.0, 0.0])
                .expect("samples");
        });
        assert_eq!(
            (heap.calls, heap.bytes),
            (0, 0),
            "steady-state GI frame paths must not allocate"
        );
    }

    #[cfg(not(feature = "window"))]
    #[test]
    fn headless_submit_names_the_window_device() {
        // The headless twin validates first, then refuses loudly with the
        // reason: no silent success without a device.
        let (bake, ms) = bake_arena_fixture().expect("fixture bakes");
        let err =
            submit_gpu([1280, 720], 12.0, [0.3, 1.0, 0.2], &bake, ms).expect_err("no device here");
        assert!(
            matches!(err, BakedGiError::Invalid(_)),
            "twin refusal stays typed"
        );
        assert!(
            format!("{err}").contains("window"),
            "refusal names the window build: {err}"
        );
        assert!(submit_gpu([0, 720], 12.0, [0.3, 1.0, 0.2], &bake, ms).is_err());
        assert!(submit_gpu([1280, 720], f32::NAN, [0.3, 1.0, 0.2], &bake, ms).is_err());
        assert!(submit_gpu([1280, 720], 12.0, [0.0, 0.0, 0.0], &bake, ms).is_err());
        assert!(submit_gpu([1280, 720], 12.0, [0.3, 1.0, 0.2], &bake, f64::NAN).is_err());
    }

    #[cfg(feature = "window")]
    fn window_device(label: &'static str) -> Option<(wgpu::Device, wgpu::Queue)> {
        crate::test_gpu::hold_gpu_slot();
        let instance = crate::backend::new_instance(wgpu::Backends::PRIMARY);
        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })) {
                Ok(a) => a,
                Err(_) => {
                    eprintln!("SKIPPED baked-gi gpu test: no wgpu adapter on this machine");
                    return None;
                }
            };
        Some(
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some(label),
                ..Default::default()
            }))
            .expect("device requests succeed when the adapter exists"),
        )
    }

    #[cfg(feature = "window")]
    #[test]
    fn gi_lightmap_upload_roundtrips() {
        // Cook bytes upload verbatim and read back bit-identical: the GPU
        // lightmap matches the CPU bake texel-for-texel, mask included.
        let (bake, _) = bake_arena_fixture().expect("fixture bakes");
        let Some((device, queue)) = window_device("rl5-gi-lightmap") else {
            return;
        };
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let texture = gpu::upload_lightmap(&device, &queue, &bake).expect("lightmap uploads");
        let (w, h) = (bake.map.w, bake.map.h);
        let row = w * LIGHTMAP_UPLOAD_BYTES_PER_TEXEL as u32;
        let padded = row.div_ceil(256) * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rl5-gi-lightmap-readback"),
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rl5-gi-lightmap-copy"),
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(enc.finish()));
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll succeeds");
        rx.recv().expect("map channel").expect("map succeeds");
        let mut expected = vec![0u8; lightmap_rgba_len(&bake).expect("len")];
        lightmap_rgba_bytes_into(&bake, &mut expected).expect("packs");
        {
            let mapped = slice.get_mapped_range().expect("mapped range");
            for (y, row_bytes) in expected.chunks_exact(row as usize).enumerate() {
                let off = y * padded as usize;
                assert_eq!(
                    &mapped[off..off + row as usize],
                    row_bytes,
                    "GPU lightmap row {y} matches the CPU bake"
                );
            }
        }
        readback.unmap();
        let scope_err = pollster::block_on(scope.pop());
        assert!(
            scope_err.is_none(),
            "zero validation errors, got {scope_err:?}"
        );
    }

    #[cfg(feature = "window")]
    #[test]
    fn gi_probe_bind_validates_and_scores() {
        // The live bind: validation-clean, bit-exact against the cook, and
        // steady-state (the second bind reuses every buffer).
        let (bake, _) = bake_arena_fixture().expect("fixture bakes");
        let Some((device, queue)) = window_device("rl5-gi-bind") else {
            return;
        };
        let mut submitter = gpu::GiSubmitter::new(&device);
        let receipt = submitter
            .submit(&device, &queue, &bake)
            .expect("bind submits");
        assert_eq!(receipt.probe_count, ProbeSpec::arena().count() as u64);
        assert!(receipt.is_clean(), "bind stays validation-clean");
        assert!(receipt.within_parity(), "device image matches the cook");
        assert_eq!(receipt.max_abs_diff, 0.0, "verbatim copy is bit-exact");
        assert_eq!(receipt.budget_ms(), GI_SUBMIT_BUDGET_MS);
        eprintln!(
            "gi_probe_bind: {:.3} ms vs {:.1} ms budget, {} probes",
            receipt.submit_ms,
            receipt.budget_ms(),
            receipt.probe_count
        );
        let created = submitter.created_buffers();
        let sized = submitter.sized_words();
        let again = submitter
            .submit(&device, &queue, &bake)
            .expect("second binds");
        assert!(again.is_clean(), "second bind stays clean");
        assert_eq!(again.probe_count, receipt.probe_count);
        assert_eq!(
            submitter.created_buffers(),
            created,
            "steady-state binds create nothing"
        );
        assert_eq!(submitter.sized_words(), sized);
    }

    // --- RL43: the linear HDR lane beside the byte lane ---------------------

    /// One bake whose `w * h` texels ramp over as many distinct grey albedos
    /// under a flat `sky` (sky == ground, no dir/point light), so texel `i`
    /// reads `sky * alp_i`. The `f32::from_bits` ramp walks 256 steps per
    /// octave over 16 octaves, one RGB9E5 code step per sample.
    fn hdr_bake(sky: f32, w: u32, h: u32) -> ArenaBake {
        let n = (w * h) as usize;
        let samples: Vec<SurfaceSample> = (0..n)
            .map(|i| {
                let bits = (((i / 256) + 114) << 23) | ((i % 256) << 15);
                let t = if n > 1 {
                    f32::from_bits(bits as u32)
                } else {
                    1.0
                };
                SurfaceSample {
                    pos: [0.0, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    albedo: [t; 3],
                    occlusion: 1.0,
                }
            })
            .collect();
        let sky = SkyAmbient {
            sky: [sky; 3],
            ground: [sky; 3],
        };
        bake_arena(
            w,
            h,
            &samples,
            &BakeScene {
                dirs: &[],
                points: &[],
                sky,
            },
            &ProbeSpec {
                nx: 1,
                ny: 1,
                nz: 1,
                min: [0.0; 3],
                step: [1.0; 3],
            },
            &[],
        )
        .expect("hdr ramp bakes")
        .0
    }

    #[test]
    fn baked_gi_gradient_holds_more_than_1024_levels() {
        // RL43: 4096 distinct input irradiance values read back over 1024
        // distinct linear levels; the byte lane can only ever show one row per
        // PALETTE entry, so it cannot carry the ramp.
        let bake = hdr_bake(4.0, 64, 64);
        let mut rows = bake.map.idx.clone();
        rows.sort_unstable();
        rows.dedup();
        assert!(rows.len() <= PALETTE_LEN, "byte lane stays on the palette");
        let uv = |k: u32| (k as f32 + 0.5) / 64.0;
        let mut levels: Vec<[u32; 3]> = (0..4096u32)
            .map(|i| {
                sample_lightmap_hdr(&bake, uv(i % 64), uv(i / 64))
                    .expect("ramp texel reads")
                    .map(f32::to_bits)
            })
            .collect();
        levels.sort_unstable();
        levels.dedup();
        assert!(levels.len() > 1024, "{} levels", levels.len());
        // The shared gate measurable agrees with what the sampler reads back.
        assert_eq!(levels.len(), hdr_texel_levels(&bake.map.tex));
    }

    #[test]
    fn baked_gi_hdr_lane_round_trips_x2() {
        let (a, _) = bake_arena_fixture().expect("first bakes");
        let (b, _) = bake_arena_fixture().expect("second bakes");
        assert_eq!(a.hdr, b.hdr, "RGB9E5 lane stable x2");
        assert_eq!(a.hdr_levels, hdr_texel_levels(&a.map.tex) as u64);
        let encoder: Vec<u32> = a.map.tex.iter().map(|t| encode_rgb9e5(*t)).collect();
        assert_eq!(a.hdr, encoder, "lane is the shared encoder");
        assert_eq!(a.texel_hdr(3, 4), b.texel_hdr(3, 4), "texel_hdr x2");
        assert_eq!(a.texel_hdr(ARENA_LIGHTMAP_W, 0), None, "off map refuses");
        assert_eq!(a.texel_hdr(0, ARENA_LIGHTMAP_H), None, "off map refuses");
        // A NaN input never stores a NaN level.
        let nan = encode_rgb9e5([f32::NAN, f32::INFINITY, -1.0]);
        assert_eq!((nan, decode_rgb9e5(nan)), (0, [0.0; 3]), "no NaN level");
        assert_eq!(hdr_texel_levels(&[[f32::NAN; 3]]), 1, "NaN is zero");
    }

    #[test]
    fn baked_gi_hdr_stays_linear_at_the_top_end() {
        // RL43: a 10000-nit texel keeps its ordering over a 1-nit texel in the
        // linear lane, while the byte lane keeps reading the palette row it
        // read before: Reinhard clamps at 4.0, so it cannot tell them apart.
        let bright = hdr_bake(10000.0, 1, 1);
        let mid = hdr_bake(100.0, 1, 1);
        let dim = hdr_bake(1.0, 1, 1);
        let hdr = |b: &ArenaBake| sample_lightmap_hdr(b, 0.5, 0.5).expect("reads");
        let (hi, me) = (hdr(&bright)[0], hdr(&mid)[0]);
        assert!(hi > 1000.0 && me > 99.0 && me < 101.0, "linear at top end");
        assert!(hi > me && me > hdr(&dim)[0] && hi / hdr(&dim)[0] > 1000.0);
        let row = sample_lightmap(&bright, 0.5, 0.5).expect("byte lane reads");
        assert_eq!(row, palette_entry(bright.map.idx[0]));
        assert_eq!(sample_lightmap(&mid, 0.5, 0.5), Some(row), "one row");
        assert!(bright.map.idx[0] > dim.map.idx[0], "byte lane orders them");
    }
}
