//! forward: the RA1 forward bind: the pinned WGSL chunk, the SH uniform and bind
//! bytes, the live-frame on/off decision and the graph compile.

use super::*;

// ---------------------------------------------------------------------------
// Forward wiring: prove the forward pass binds the IBL chunk instead of
// only shipping it. The chart pass below already samples the full bake on a
// device; the forward pipeline must inline the same sampling chunk and bind
// the same prefilter, BRDF and SH images every frame.
// ---------------------------------------------------------------------------

/// Markers a compiled forward WGSL must contain to sample the IBL bake: the
/// prefiltered cubemap, the BRDF LUT, the sampler, the SH uniform and the
/// sampling entry. The window-only `gpu::FORWARD_IBL_WGSL` chunk carries
/// all five; the forward pipeline inlines that chunk when the material bind
/// group lands.
pub const FORWARD_IBL_MARKERS: [&str; 5] = [
    "ibl_prefilter",
    "ibl_brdf",
    "ibl_sampler",
    "ibl_sh",
    "ibl_sample",
];

/// Assert a compiled forward WGSL binds the IBL chunk.
///
/// Owner: render lane; consumer: the forward presenter pipeline plus the
/// wiring gate. Units: none (string markers). Threading: pure with no IO,
/// locks or clock; safe on any thread. Determinism: the same string gives
/// the same verdict on every run. Errors: [`IblError::Invalid`] naming the
/// first missing marker; empty shaders refuse instead of passing silently.
///
/// ```
/// use engine_render::ibl::{assert_forward_wgsl_binds_ibl, FORWARD_IBL_MARKERS};
/// let stitched = FORWARD_IBL_MARKERS.join("\n");
/// assert!(assert_forward_wgsl_binds_ibl(&stitched).is_ok());
/// ```
pub fn assert_forward_wgsl_binds_ibl(forward_wgsl: &str) -> Result<(), IblError> {
    for marker in FORWARD_IBL_MARKERS {
        if !forward_wgsl.contains(marker) {
            return Err(IblError::Invalid(format!(
                "forward pass misses IBL bind marker '{marker}': inline FORWARD_IBL_WGSL"
            )));
        }
    }
    Ok(())
}

// Accept CRLF checkouts as well as LF (`core.autocrlf` on Windows).
pub(crate) fn fs_close(rest: &str) -> Option<usize> {
    rest.find("\n}\n").or_else(|| rest.find("\r\n}\r\n"))
}

/// Assert the live window forward scales baked diffuse by occlusion only.
///
/// Owner: render lane; consumer: the EG12 wiring gate plus the live forward
/// frame through the same rule. Units: none (string scan). Threading: pure
/// with no IO, locks or clock; safe on any thread. Determinism: the same
/// string gives the same verdict on every run. Errors: [`IblError::Invalid`]
/// when the window `fs` is missing, when it multiplies the combined `ibl`
/// by occlusion, when no diffuse-by-occlusion scale is found, or when the
/// open specular term is missing. Mirrors CPU [`ambient_occluded`] plus
/// `shade_ibl_occluded` (diffuse times occlusion, specular open).
pub fn assert_forward_ibl_scales_diffuse_only(forward_wgsl: &str) -> Result<(), IblError> {
    let start = forward_wgsl.find("fn fs(in: VOut)").ok_or_else(|| {
        IblError::Invalid("forward pass misses window fs: inline FORWARD_IBL_WGSL".into())
    })?;
    let rest = &forward_wgsl[start..];
    let end = fs_close(rest).ok_or_else(|| {
        IblError::Invalid("forward fs misses closing brace: inline FORWARD_IBL_WGSL".into())
    })?;
    let body = &rest[..end];
    if body.contains("ibl * occlusion")
        || body.contains("ibl*occlusion")
        || body.contains("ibl *occlusion")
        || body.contains("ibl* occlusion")
    {
        return Err(IblError::Invalid(
            "forward fs scales combined ibl by occlusion: scale the diffuse irradiance by occlusion and leave specular open".into(),
        ));
    }
    let has_diffuse = body.contains("ibl_diffuse") || body.contains("ibl_irradiance");
    let scales = body.contains("* occlusion") || body.contains("*occlusion");
    if !(has_diffuse && scales) {
        return Err(IblError::Invalid(
            "forward fs misses diffuse-by-occlusion scale: scale ibl_diffuse by occlusion".into(),
        ));
    }
    if !body.contains("ibl_spec") {
        return Err(IblError::Invalid(
            "forward fs misses open specular: keep ibl_specular unscaled by occlusion".into(),
        ));
    }
    Ok(())
}

/// Live-frame IBL decision: sample the bake only when the forward pass is
/// bound and the bake carries light; otherwise keep the flat floor.
///
/// Owner: render lane; consumer: the live forward frame (timing lands next;
/// this slice names the rule only). Units: none (two booleans in, one out).
/// Threading: pure with no IO, locks, clock, allocation or device; safe on
/// any thread. Determinism: the same inputs give the same verdict on every
/// run. Fail-closed: unbound forward keeps the floor even with a bright
/// bake; a zero bake keeps the floor even when bound. Bound means
/// [`assert_forward_wgsl_binds_ibl`] on the shipped forward file is ok;
/// light means the bake carries nonzero radiance; zero means the zero
/// fixture.
pub fn live_ibl_should_sample(forward_binds_ibl: bool, bake_has_light: bool) -> bool {
    forward_binds_ibl && bake_has_light
}

/// Bake carries light: the missing accessor the live decision needs.
///
/// Owner: render lane; consumer: `shade_ibl` (exact-zero early-out below)
/// plus the live forward frame through [`live_ibl_should_sample`].
/// Units: linear radiance in `source_average`, SH and cube texels; out is a
/// verdict. Threading: pure with no IO, locks, clock, allocation or device;
/// safe on any thread. Determinism: the same bake gives the same verdict on
/// every run. Fail-closed: non-finite or non-positive radiance reads as no
/// light, so the frame keeps the flat floor. Light means a finite channel
/// above 1e-6 in `source_average` and in the SH coefficients and in the
/// prefilter cube texels; zero means the zero fixture. The analytic BRDF
/// LUT is excluded: it stays non-zero even for a black HDRI.
#[cfg(feature = "v1")]
pub fn bake_has_light(bake: &IblBake) -> bool {
    let source_lit = bake
        .source_average
        .iter()
        .any(|c| c.is_finite() && *c > 1e-6);
    if !source_lit {
        return false;
    }
    let sh_lit = bake
        .sh
        .coeffs
        .iter()
        .flat_map(|c| c.iter())
        .any(|c| c.is_finite() && *c > 1e-6);
    if !sh_lit {
        return false;
    }
    bake.cube
        .faces
        .iter()
        .flat_map(|t| t.iter())
        .any(|c| c.is_finite() && *c > 1e-6)
}

/// Live on-vs-off diffuse gap for one albedo probe: baked `ibl_diffuse`
/// against the flat `AMBIENT_FLOOR` path, as a single max-channel gap.
///
/// Owner: render lane; consumer: `present/gpu_present/live.rs`
/// `live_ibl_should_count` plus the live forward frame that samples the
/// same bake. Units: linear RGB gap for dielectric `albedo` in `0..=1`
/// with `normal` unit. Threading: pure with no IO, locks, clock or device;
/// safe on any thread. Determinism: the same bake and inputs give the same
/// gap on every run. Errors: [`IblError::Invalid`] when the bake carries
/// no light (same rule as [`bake_has_light`]), when `normal` is not a
/// finite nonzero direction via `eval_sh`, or when `albedo` is not finite
/// `0..=1`. Mirrors `ibl_diffuse` in `shaders/forward.wgsl` with metallic
/// 0 (dielectric); init-time oracle only, never per frame, never hashed.
#[cfg(feature = "v1")]
pub fn live_ibl_on_off_gap(
    bake: &IblBake,
    normal: [f32; 3],
    albedo: [f32; 3],
) -> Result<f32, IblError> {
    if !bake_has_light(bake) {
        return Err(IblError::Invalid(
            "ibl live gap bake carries no light: keeps the flat floor".into(),
        ));
    }
    if !albedo.iter().all(|c| c.is_finite()) {
        return Err(IblError::Invalid("ibl gap albedo is not finite".into()));
    }
    if albedo.iter().any(|c| *c < 0.0 || *c > 1.0) {
        return Err(IblError::Invalid(format!(
            "ibl gap albedo {albedo:?} outside 0..=1"
        )));
    }
    let e = eval_sh(&bake.sh, normal)?;
    let mut gap = 0.0f32;
    for c in 0..3 {
        let on = albedo[c] * e[c] / PI;
        let off = albedo[c] * IBL_OFF_FLOOR[c];
        gap = gap.max((on - off).abs());
    }
    Ok(gap)
}

/// Live-frame IBL receipt: CPU wall-clock ms for the forward IBL-bound
/// encode, reported as `ibl.2` submit us on the live receipt.
///
/// Owner: render lane; consumer: `present_frame_live` in
/// `present/gpu_present/live.rs` with the measured encode time. Units: ms in,
/// pixels for sizes, microseconds out as `(passes, errors, submit_us)`.
/// Threading: pure with no IO, locks, clock, allocation or device; safe on
/// any thread. Determinism: the same inputs give the same tuple on every run.
/// Fail-closed: zero-size stays skipped at `(0, 0, 0)`; a live encode floors
/// to 1 us so the per-pass table shows `ibl` above 0 ms when the forward
/// bound the bake. `passes` stays 0: it counts the chart tranche, and the
/// live IBL rides forward; `errors` stays 0 on live frames.
pub fn live_ibl_receipt(ibl_ms: f64, width: u32, height: u32) -> (u32, u32, u64) {
    if width == 0 || height == 0 {
        return (0, 0, 0);
    }
    let floored = if !ibl_ms.is_finite() || ibl_ms <= 0.0 {
        0.001
    } else {
        ibl_ms.max(0.001)
    };
    if floored >= 1.0e15 {
        return (0, 0, u64::MAX);
    }
    // Mirrors `Presenter::ms_to_us` so the window receipt reads the same
    // 1 us floor; presentation timing only, never hashed.
    let us = u64::try_from(std::time::Duration::from_secs_f64(floored / 1000.0).as_micros())
        .unwrap_or(u64::MAX);
    (0, 0, us.max(1))
}

// ---------------------------------------------------------------------------
// Forward IBL bind (headless): the sampling chunk plus the SH uniform image
// the forward pass binds. These carry no wgpu types, so they live outside
// `mod gpu`: the headless bind lock below hashes and packs them with no
// device, and `mod gpu` re-exports both for the live submit path. New module
// edge: `graph` reads `FORWARD_IBL_INPUTS` for the bake -> forward wiring.
// ---------------------------------------------------------------------------

/// Forward-pass IBL sampling chunk (WGSL): prefiltered cubemap sample
/// plus SH irradiance dot plus BRDF LUT scale/bias. Engine-owned code
/// (no port markers); the forward pipeline inlines this chunk when
/// the material bind group lands. The SH basis matches
/// [`crate::gi::sh_basis`] term for term; coefficients pack as 9 vec4s
/// (RGB in xyz, w pads) so the GPU dot matches `eval_sh`.
pub const FORWARD_IBL_WGSL: &str = r#"
@group(1) @binding(0) var ibl_prefilter: texture_cube<f32>;
@group(1) @binding(1) var ibl_brdf: texture_2d<f32>;
@group(1) @binding(2) var ibl_sampler: sampler;
@group(1) @binding(3) var<uniform> ibl_sh: array<vec4<f32>, 9>;
fn ibl_sh_basis(d: vec3<f32>) -> array<f32, 9> {
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
fn ibl_irradiance(n: vec3<f32>) -> vec3<f32> {
    let b = ibl_sh_basis(n);
    var e = vec3<f32>(0.0, 0.0, 0.0);
    e += ibl_sh[0].xyz * b[0];
    e += ibl_sh[1].xyz * b[1];
    e += ibl_sh[2].xyz * b[2];
    e += ibl_sh[3].xyz * b[3];
    e += ibl_sh[4].xyz * b[4];
    e += ibl_sh[5].xyz * b[5];
    e += ibl_sh[6].xyz * b[6];
    e += ibl_sh[7].xyz * b[7];
    e += ibl_sh[8].xyz * b[8];
    return max(e, vec3<f32>(0.0, 0.0, 0.0));
}
const MS_E_FLOOR: f32 = 0.25;
fn ms_gain(f0: vec3<f32>, e: f32) -> vec3<f32> {
    return vec3<f32>(1.0) + f0 * (1.0 / max(e, MS_E_FLOOR) - 1.0);
}
fn ibl_sample(n: vec3<f32>, v: vec3<f32>, f0: vec3<f32>, roughness: f32, albedo: vec3<f32>, metallic: f32) -> vec3<f32> {
    let ndotv = clamp(dot(n, v), 0.0, 1.0);
    let r = reflect(-v, n);
    let lod = roughness * 4.0;
    let pre = textureSampleLevel(ibl_prefilter, ibl_sampler, r, lod).rgb;
    let fab = textureSampleLevel(ibl_brdf, ibl_sampler, vec2<f32>(ndotv, roughness), 0.0).rg;
    let diffuse = (1.0 - metallic) * albedo * ibl_irradiance(n) / 3.141592653589793;
    // RL47: the same multiple-scatter gain the forward lobe applies; see
    // `ibl_specular` in `shaders/forward.wgsl`.
    let comp = ms_gain(f0, fab.x + fab.y);
    return diffuse + pre * (f0 * fab.x + vec3<f32>(fab.y, fab.y, fab.y)) * comp;
}
"#;

/// Graph hash of the bound forward chunk as a single node, pinned at cook
/// review time. The bind lock asserts this hash, so the bound chunk is
/// exactly the reviewed bytes: any drift fails loudly instead of rebinding
/// silently. Hashed through [`crate::graph::Graph::graph_hash`] (the
/// crate's hash owner) so the algorithm is owned once.
pub const FORWARD_IBL_CHUNK_HASH: u64 = 13619901356611120678;

/// Graph resources the forward pass reads from the IBL bake: prefiltered
/// cubemap, BRDF LUT and SH uniform. Owned here so the graph names the same
/// strings the bind carries; [`compile_ibl_for_forward`] wires them.
pub const FORWARD_IBL_INPUTS: [&str; 3] = ["ibl_prefilter", "ibl_brdf", "ibl_sh"];
/// Bake pass name producing [`FORWARD_IBL_INPUTS`] in
/// [`compile_ibl_for_forward`].
pub const IBL_BAKE_PASS: &str = "ibl-bake";
/// Forward pass name consuming [`FORWARD_IBL_INPUTS`] (matches the arena
/// fixture forward name, but this wiring compiles its own mini-graph).
pub const IBL_FORWARD_PASS: &str = "forward";
/// SH uniform image size in bytes: 9 coefficients as vec4s (RGB plus pad).
pub const FORWARD_IBL_SH_UNIFORM_BYTES: usize = 144;
/// Compiled forward bind image size in bytes: 8-byte chunk hash plus the
/// [`FORWARD_IBL_SH_UNIFORM_BYTES`]-byte SH uniform.
pub const FORWARD_IBL_BIND_BYTES: usize = 152;

/// Pack the SH uniform image into caller-owned storage (exactly 144 bytes):
/// 9 RGB coefficients as vec4s (RGB in xyz, w pads), same order as
/// [`IrradianceSh::coeffs`], so the GPU dot matches `eval_sh` term for
/// term. Slices in, slices out: steady-state submits pack into a reused
/// scratch uniform and grow nothing.
pub fn sh_uniform_bytes_into(sh: &IrradianceSh, out: &mut [u8; 144]) {
    for (i, c) in sh.coeffs.iter().enumerate() {
        let base = i * 16;
        out[base..base + 4].copy_from_slice(&c[0].to_le_bytes());
        out[base + 4..base + 8].copy_from_slice(&c[1].to_le_bytes());
        out[base + 8..base + 12].copy_from_slice(&c[2].to_le_bytes());
        out[base + 12..base + 16].copy_from_slice(&0.0f32.to_le_bytes());
    }
}

/// Compiled forward bind image (headless): bytes `[0..8]` carry
/// [`FORWARD_IBL_CHUNK_HASH`] little-endian, bytes `[8..152]` carry the SH
/// uniform from [`sh_uniform_bytes_into`]. Assembled into caller-owned
/// storage only, so steady-state repacks allocate nothing; the same SH gives
/// the same bytes on every run. Infallible: every SH packs.
pub fn forward_ibl_bind_image(sh: &IrradianceSh, out: &mut [u8; FORWARD_IBL_BIND_BYTES]) {
    out[0..8].copy_from_slice(&FORWARD_IBL_CHUNK_HASH.to_le_bytes());
    let mut uniform = [0u8; FORWARD_IBL_SH_UNIFORM_BYTES];
    sh_uniform_bytes_into(sh, &mut uniform);
    out[8..FORWARD_IBL_BIND_BYTES].copy_from_slice(&uniform);
}

/// Bake -> forward wiring for the IBL bind (headless): producer
/// [`IBL_BAKE_PASS`] outputs [`FORWARD_IBL_INPUTS`], consumer
/// [`IBL_FORWARD_PASS`] reads them, and compile infers the handover edges,
/// so the compiled graph carries one Transition barrier per resource from
/// bake to forward. Mirrors fog's `compile_fog_for_forward`: planning-time
/// only (the plan allocates once); per-submit packing stays on
/// [`forward_ibl_bind_image`] with caller-owned storage. Fails loudly with
/// [`crate::graph::GraphError`] when the wiring stops compiling.
pub fn compile_ibl_for_forward() -> Result<crate::graph::CompiledGraph, crate::graph::GraphError> {
    let mut graph = crate::graph::FrameGraph::new();
    let bake = graph.add_pass(
        crate::graph::PassDesc::new(IBL_BAKE_PASS)
            .output(FORWARD_IBL_INPUTS[0])
            .output(FORWARD_IBL_INPUTS[1])
            .output(FORWARD_IBL_INPUTS[2])
            .work(1, 4),
    );
    let forward = graph.add_pass(
        crate::graph::PassDesc::new(IBL_FORWARD_PASS)
            .input(FORWARD_IBL_INPUTS[0])
            .input(FORWARD_IBL_INPUTS[1])
            .input(FORWARD_IBL_INPUTS[2])
            .work(2, 6),
    );
    graph.add_edge(bake, forward)?;
    graph.compile()
}

// ---------------------------------------------------------------------------
// GPU descriptors behind `window`: real texture allocation for the bake.
// Allocation tests skip without an adapter, exactly like the presenter
// tests. The forward chunk plus the SH pack live above (no wgpu types) and
// are re-exported here for the live submit path.
// ---------------------------------------------------------------------------
