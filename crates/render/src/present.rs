//! present: windowed presenter over a draw list (RA1).
//!
//! Owner: render lane, consumer WIN1 window loop and ED1 viewport next.
//! Units: pixels for sizes, GPU handles for cached meshes/textures. One frame
//! is forward PBR draws into a linear HDR target (Rgba16Float, exposure
//! only) plus depth, then one blit to the swapchain twin that does the
//! frame's only tone map, ACES plus sRGB. The live window frame never waits
//! on the GPU: height fog is a per-fragment function of the forward shader
//! (`fog_fwd`), the sky is a far-plane draw, and the blocking look probes
//! (bind probes, IBL chart, depth readback with fog and GTAO) run only under
//! the opt-in `Presenter::set_look_probes` or on the offscreen capture twin.
//! The forward pass is the one shader file `shaders/forward.wgsl`
//! (`v1_gpu::FORWARD_PBR_WGSL`, entry points `vs` and `fs`): sun-shadow
//! cascade at group 0 binding 3, base-color sample at bindings 4-5, fog at
//! binding 6, no string patching. Resize/minimize/Outdated/Lost keep the
//! last-good pipeline and targets; every failure is a typed `PresentError` with a recovery action.

// ---------------------------------------------------------------------------
// Bounds: every external size is validated before allocation.
// ---------------------------------------------------------------------------

/// Largest accepted frame dimension; matches `v1::MAX_CAPTURE_DIM` (the GPU
/// path asserts equality in debug so the two never drift).
pub const MAX_FRAME_DIM: u32 = 2048;
/// Largest accepted texture dimension (same bound as frames).
pub const MAX_TEXTURE_DIM: u32 = 2048;
/// Most draws in one frame; the draw list is rejected above this.
pub const MAX_DRAWS_PER_FRAME: usize = 256;
/// Most GPU meshes resident; insertion-order eviction above this.
pub const MAX_CACHED_MESHES: usize = 512;
/// Most GPU textures resident; insertion-order eviction above this.
pub const MAX_CACHED_TEXTURES: usize = 512;

pub mod aa;
pub mod cache;
#[cfg(feature = "window")]
pub mod fog_fwd;
pub mod look;
pub mod surface;
pub mod wait_stats;

pub use aa::{
    compile_taa_off_at_controller_scale, select_aa, shimmer_ab, AaChoice, AaFrameManifest,
    ShimmerAb,
};
pub use cache::{
    check_texture_resident, missing_texture_error, CpuCache, MeshKey, TextureKey, TextureUpload,
};
pub use look::{
    bake_look_on_worker, baked_gi_frame_sample, baked_gi_receipt_for_frame,
    compile_fog_for_presenter, contact_for_frame, fog_blit_uniform_bytes, fog_err, fog_fits_frame,
    fog_params_for_capture, fog_params_for_look_bake, ibl_ambient_for_capture,
    look_bake_for_capture, rl8_err, skybox_sample_for_capture, FOG_BLIT_UNIFORM_BYTES,
    RL9_CONTACT_INTENSITY, RL9_CONTACT_RADIUS_PX, RL9_CONTACT_THICKNESS_M, RL9_FOG_BASE_M,
    RL9_FOG_DENSITY_PER_M, RL9_FOG_EXPOSURE_EV100, RL9_FOG_FALLOFF_M, RL9_FOG_MAX_DIST_M,
};
pub use surface::{
    surface_action, surface_fault_code, PresentError, RecoveryAction, SizeClass, SurfaceAction,
    SurfaceEvent,
};
pub use wait_stats::{
    percentiles, CreateTally, FrameKind, FrameWaits, GpuTimingReport, Percentiles, WaitStats,
};

/// One frame of fog inputs decoded from the RA1 depth target: view distance
/// plus world height plus facing normals, all row-major over the swap size.
/// Built from the depth readback on probe frames only (the opt-in look
/// probes and the offscreen capture twin); the live window frame never
/// decodes one. The `FogSubmitter` uploads these into the fog and contact
/// compute passes.
#[derive(Clone, PartialEq, Debug)]
pub struct DepthFeed {
    /// Frame width in pixels.
    pub width_px: u32,
    /// Frame height in pixels.
    pub height_px: u32,
    /// View distance in meters (Euclidean eye-to-surface), positive finite.
    pub depth_m: Vec<f32>,
    /// World height in meters (world Y), finite.
    pub height_m: Vec<f32>,
    /// Facing unit normals (toward the eye), finite nonzero.
    pub normals: Vec<[f32; 3]>,
}

impl DepthFeed {
    /// Texel count (`width * height`).
    pub fn texels(&self) -> usize {
        self.depth_m.len()
    }
}

/// Decode one depth readback into the fog feed: unproject each texel through
/// the inverse view-projection (column-major, `glam` `to_cols_array` order,
/// the same image the uniforms carry), take the Euclidean eye distance plus
/// world Y, then difference the world grid for facing normals.
///
/// Row 0 is NDC y `+1` (top), matching the `v1` CPU rasterizer row map, so
/// heights agree with the capture the parity tests compare against. Cleared
/// (sky) texels unproject onto the far plane: finite, distant, heavily
/// fogged by the bake. Fails loudly (`Readback`, retry the frame) on ragged
/// input, out-of-range depth or a degenerate matrix: the fog submit never
/// sees a half-decoded feed.
///
/// Probe path only: the live window frame has no depth readback and never
/// calls this (fog is a per-fragment function in the forward shader, see
/// the `fog_fwd` module). It allocates four buffers per call (`world`,
/// `depth_m`, `height_m`, `normals`), a 4-alloc floor proved by the
/// counting-allocator gate in `sl21_submit_audit` (pattern:
/// `docs/library/counting-allocator-floors.md`).
pub fn decode_depth_feed(
    raw_depth: &[f32],
    width_px: u32,
    height_px: u32,
    inv_view_proj_cols: [[f32; 4]; 4],
    eye: [f32; 3],
) -> Result<DepthFeed, PresentError> {
    if SizeClass::classify(width_px, height_px) != SizeClass::Sizes {
        return Err(PresentError::InvalidSize {
            width: width_px,
            height: height_px,
        });
    }
    let w = width_px as usize;
    let h = height_px as usize;
    let n = w * h;
    if raw_depth.len() != n {
        return Err(PresentError::Readback(format!(
            "rl9 depth length {} mismatches frame {width_px}x{height_px}",
            raw_depth.len()
        )));
    }
    for col in &inv_view_proj_cols {
        for c in col {
            if !c.is_finite() {
                return Err(PresentError::Readback(
                    "rl9 camera inverse is not finite".into(),
                ));
            }
        }
    }
    for c in &eye {
        if !c.is_finite() {
            return Err(PresentError::Readback(
                "rl9 camera eye is not finite".into(),
            ));
        }
    }
    let eye64 = [eye[0] as f64, eye[1] as f64, eye[2] as f64];
    let (w64, h64) = (w as f64, h as f64);
    let mut world = vec![[0.0f64; 3]; n];
    let mut depth_m = vec![0.0f32; n];
    let mut height_m = vec![0.0f32; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let d = raw_depth[i];
            if !d.is_finite() || !(0.0..=1.0).contains(&d) {
                return Err(PresentError::Readback(format!(
                    "rl9 depth texel {i} value {d} outside 0..=1"
                )));
            }
            let nx = 2.0 * (x as f64 + 0.5) / w64 - 1.0;
            let ny = 1.0 - 2.0 * (y as f64 + 0.5) / h64;
            let clip = [nx, ny, d as f64, 1.0];
            let mut q = [0.0f64; 4];
            for r in 0..4 {
                q[r] = inv_view_proj_cols[0][r] as f64 * clip[0]
                    + inv_view_proj_cols[1][r] as f64 * clip[1]
                    + inv_view_proj_cols[2][r] as f64 * clip[2]
                    + inv_view_proj_cols[3][r] as f64 * clip[3];
            }
            if !q.iter().all(|v| v.is_finite()) || q[3] <= 0.0 {
                return Err(PresentError::Readback(format!(
                    "rl9 depth texel {i} unprojects degenerately"
                )));
            }
            let px = q[0] / q[3];
            let py = q[1] / q[3];
            let pz = q[2] / q[3];
            let dx = px - eye64[0];
            let dy = py - eye64[1];
            let dz = pz - eye64[2];
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            if !dist.is_finite() || dist <= 0.0 {
                return Err(PresentError::Readback(format!(
                    "rl9 depth texel {i} decodes to non-positive distance"
                )));
            }
            world[i] = [px, py, pz];
            depth_m[i] = dist as f32;
            height_m[i] = py as f32;
        }
    }
    // Facing normals from central differences of the world grid, oriented
    // toward the eye; a degenerate neighborhood falls back to the view ray
    // (nonzero by the distance check above), so every normal the fog
    // contact bake re-normalizes is finite and nonzero.
    let mut normals = vec![[0.0f32; 3]; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let xl = if x == 0 { 0 } else { x - 1 };
            let xr = if x + 1 >= w { w - 1 } else { x + 1 };
            let yt = if y == 0 { 0 } else { y - 1 };
            let yb = if y + 1 >= h { h - 1 } else { y + 1 };
            let a = world[y * w + xr];
            let b = world[y * w + xl];
            let c = world[yb * w + x];
            let e = world[yt * w + x];
            let tx = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
            let ty = [c[0] - e[0], c[1] - e[1], c[2] - e[2]];
            let mut nrm = [
                tx[1] * ty[2] - tx[2] * ty[1],
                tx[2] * ty[0] - tx[0] * ty[2],
                tx[0] * ty[1] - tx[1] * ty[0],
            ];
            let to_eye = [
                eye64[0] - world[i][0],
                eye64[1] - world[i][1],
                eye64[2] - world[i][2],
            ];
            if nrm[0] * to_eye[0] + nrm[1] * to_eye[1] + nrm[2] * to_eye[2] < 0.0 {
                nrm = [-nrm[0], -nrm[1], -nrm[2]];
            }
            let len2 = nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2];
            let unit = if len2 < 1e-24 {
                let l =
                    (to_eye[0] * to_eye[0] + to_eye[1] * to_eye[1] + to_eye[2] * to_eye[2]).sqrt();
                [to_eye[0] / l, to_eye[1] / l, to_eye[2] / l]
            } else {
                let l = len2.sqrt();
                [nrm[0] / l, nrm[1] / l, nrm[2] / l]
            };
            normals[i] = [unit[0] as f32, unit[1] as f32, unit[2] as f32];
        }
    }
    Ok(DepthFeed {
        width_px,
        height_px,
        depth_m,
        height_m,
        normals,
    })
}

// ---------------------------------------------------------------------------
// Pipeline warmup: zero first-use stutter — scene variant keys plus the
// camera walk.
//
// Owner: render lane; consumer is the window presenter load path (warms every
// scene variant key before the first frame) and the per-frame walk probe
// (`get` + `poll_ready` only, never compiles). Units: key counts; walk cost
// is a multiple of the median frame (1.0 warm, 2.0 on a creation hitch).
// Determinism: keys enumerate the shipped forward-PBR manifest rows in name
// order, so identical inputs give identical key sets on any host; the walk
// report is pure counts, never wall clock. Errors: none here — warm
// failures count as `cache::WarmSummary.failed` with a capped diagnostic on
// the load summary. Budget: key enumeration allocates once at load; the
// per-frame walk borrows and allocates nothing.
// ---------------------------------------------------------------------------

/// Opaque pipeline-layout id folded into every scene variant key. The
/// forward pass is one layout, so one id covers the shipped set; it
/// matches the proven warmup value.
pub const OPT19_PIPELINE_LAYOUT: u64 = 7;

/// Stable shader identity for the headless scene key set. The window build
/// warms the same define set over the real `FORWARD_PBR_WGSL` bytes through
/// [`scene_variant_keys_for`]; the headless twin warms this marker so the
/// walk shape is tested with no GPU.
pub const OPT19_SHADER_BYTES: &[u8] = b"forward-pbr-shader";

/// Scene variant keys for explicit shader bytes: the shipped forward-PBR
/// manifest rows in name order, each folded with `layout` into a
/// [`crate::cache::PipelineKey`]. The presenter load path passes these to
/// [`crate::cache::PipelineCache::warm_keys`] before the first frame, so a
/// camera walk across every material never compiles mid-frame. Load-path
/// only: allocates the key set once.
///
/// ```
/// let keys = engine_render::present::scene_variant_keys_for(b"forward-pbr-shader", 7);
/// assert_eq!(keys.len(), 4);
/// ```
pub fn scene_variant_keys_for(shader_bytes: &[u8], layout: u64) -> Vec<crate::cache::PipelineKey> {
    let manifest = crate::variants::VariantManifest::shipped_forward_pbr();
    let mut names: Vec<&str> = manifest.names().collect();
    names.sort_unstable();
    names
        .iter()
        .filter_map(|name| {
            let variant = match manifest.get(name) {
                Some(variant) => variant,
                None => {
                    debug_assert!(false, "shipped manifest lists every enumerated name");
                    return None;
                }
            };
            let defines: Vec<(&str, &str)> = variant
                .defines
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect();
            Some(crate::cache::PipelineKey::new(
                shader_bytes,
                &defines,
                layout,
            ))
        })
        .collect()
}

/// Headless scene variant keys over [`OPT19_SHADER_BYTES`] with
/// [`OPT19_PIPELINE_LAYOUT`]: the four shipped forward-PBR variants. The
/// window build calls [`scene_variant_keys_for`] with the real forward
/// shader bytes instead; both warm through the same manifest rows.
/// Load-path only: allocates the key set once.
pub fn scene_variant_keys() -> Vec<crate::cache::PipelineKey> {
    scene_variant_keys_for(OPT19_SHADER_BYTES, OPT19_PIPELINE_LAYOUT)
}

/// One camera walk over the warmed scene keys: frames stepped, pipeline
/// creations observed, worst frame cost as a multiple of the median frame.
/// A creation hitch doubles its frame (2.0); a fully warm walk posts 1.0,
/// under the 1.5x bar by construction. Pure counts, never wall clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WalkReport {
    /// Frames stepped (one per scene variant key).
    pub frames: usize,
    /// Pipelines the walk had to create (0 warm; every key missed cold).
    pub creations: usize,
    /// Worst frame cost as a multiple of the median: 1.0 warm, 2.0 on any
    /// creation hitch.
    pub max_over_median: f32,
}

impl WalkReport {
    /// True when no walk frame exceeds 1.5x the median from creations.
    pub fn passes(&self) -> bool {
        self.max_over_median < 1.5
    }
}

/// Walk every scene key through `get` + `poll_ready` only: the per-frame
/// shape. Never compiles, inserts or allocates; misses count as creations
/// so a cold walk fails the bar loudly instead of warming silently.
pub fn walk_pipelines(
    cache: &crate::cache::PipelineCache,
    keys: &[crate::cache::PipelineKey],
) -> WalkReport {
    let mut creations = 0usize;
    for key in keys {
        let hit = cache.get(key).is_some()
            && matches!(cache.poll_ready(key), crate::cache::PollState::Ready { .. });
        if !hit {
            creations += 1;
        }
    }
    WalkReport {
        frames: keys.len(),
        creations,
        max_over_median: if creations == 0 { 1.0 } else { 2.0 },
    }
}

// ---------------------------------------------------------------------------
// Draw list over validated v1 scene data: one camera plus light per frame,
// one material per draw. Multi-draw is one render pass with one bind group
// per draw over the shared `v1_gpu` pipeline.
// ---------------------------------------------------------------------------

/// One draw: a cached mesh plus its PBR material, optionally modulated by
/// a resident base-color texture (textured draws).
#[cfg(feature = "v1")]
#[derive(Clone, PartialEq, Debug)]
pub struct DrawItem {
    /// Resident mesh key; unknown keys fail the frame with `InvalidDraw`.
    pub mesh: MeshKey,
    /// Metallic-roughness factors for this draw; multiplied by the sampled
    /// `base_color` texel when one is bound.
    pub material: crate::v1::PbrMaterial,
    /// Optional resident base-color texture (sRGB bytes, sampled back to
    /// linear): the forward pass multiplies the material factor by the
    /// sampled texel. `None` binds the 1x1 white fallback, so untextured
    /// draws render byte-identical pixels to before. Unknown keys fail the
    /// frame with `InvalidDraw`, mirroring meshes.
    pub base_color: Option<TextureKey>,
}

/// One frame of draws: shared camera plus light, per-draw mesh plus material.
#[cfg(feature = "v1")]
#[derive(Clone, PartialEq, Debug)]
pub struct DrawList {
    /// Frame camera; the frame size overrides its aspect.
    pub camera: crate::v1::Camera,
    /// The one directional light.
    pub light: crate::v1::DirectionalLight,
    /// Draws in submission order; empty draws a clear-only frame.
    pub items: Vec<DrawItem>,
}

#[cfg(feature = "v1")]
impl DrawList {
    /// Validate draw count plus every material; mesh residency is checked at
    /// render time so the error can name the missing key.
    pub fn validate(&self) -> Result<(), PresentError> {
        if self.items.len() > MAX_DRAWS_PER_FRAME {
            return Err(PresentError::InvalidDraw {
                index: self.items.len(),
                reason: format!("draw count exceeds {MAX_DRAWS_PER_FRAME}"),
            });
        }
        for (index, item) in self.items.iter().enumerate() {
            item.material
                .validate()
                .map_err(|e| PresentError::InvalidDraw {
                    index,
                    reason: format!("material: {e}"),
                })?;
        }
        Ok(())
    }
}

#[cfg(feature = "window")]
mod gpu_present;

#[cfg(feature = "window")]
pub use gpu_present::*;

#[cfg(test)]
mod tests_gpu;

#[cfg(test)]
mod tests_linear;

#[cfg(test)]
mod tests_unit;
