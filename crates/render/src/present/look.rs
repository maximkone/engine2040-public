use super::surface::{PresentError, SizeClass};

// ---------------------------------------------------------------------------
// RL9: fog depth-feed production wiring.
//
// Owner: render lane; consumer WIN1 window loop through `render_offscreen`
// and `render_surface` (ED1 viewport rides the same calls). Units: meters
// for depth/height, pixels for sizes. Determinism: row-major integer order,
// f64 accumulation; the same depth buffer plus camera gives bit-identical
// buffers twice. Errors: feed-shape problems fail as `PresentError` before
// any GPU work; fog device failures map through `fog_err`. Budget: the
// decode is one pass over the frame; `FogSubmitter` sizes only on shape
// change; per-frame scratch is reused. `FOG_BUDGET_MS` (0.2) plus
// `CONTACT_BUDGET_MS` (0.3) bound the device passes and the receipt quotes
// submit us against both bars.
// ---------------------------------------------------------------------------

/// RL9 feed fog density per meter (mirrors [`crate::fog::FogParams::day`]).
pub const RL9_FOG_DENSITY_PER_M: f32 = 0.02;
/// RL9 feed height falloff in meters (mirrors `FogParams::day`).
pub const RL9_FOG_FALLOFF_M: f32 = 8.0;
/// RL9 feed fog base height in meters (mirrors `FogParams::day`).
pub const RL9_FOG_BASE_M: f32 = 0.0;
/// RL9 feed fog cutoff distance in meters (mirrors `FogParams::day`).
pub const RL9_FOG_MAX_DIST_M: f32 = 500.0;
/// RL9 feed exposure anchor in EV100 (matches the look-frame anchor).
pub const RL9_FOG_EXPOSURE_EV100: f32 = 12.0;
/// RL9 feed contact radius in pixels
/// (mirrors [`crate::fog::ContactConfig::small`]).
pub const RL9_CONTACT_RADIUS_PX: f32 = 6.0;
/// RL9 feed contact thickness in meters (mirrors `ContactConfig::small`).
pub const RL9_CONTACT_THICKNESS_M: f32 = 0.1;
/// RL9 feed contact intensity (mirrors `ContactConfig::small`).
pub const RL9_CONTACT_INTENSITY: f32 = 0.9;
/// Byte size of the packed fog-composite uniform (linear color + frame size).
pub const FOG_BLIT_UNIFORM_BYTES: usize = 32;

/// Map a fog failure into the presenter error (mirrors the sky/layer/
/// particle/decal seams above the GPU presenter).
pub fn fog_err(context: &str, e: crate::fog::FogError) -> PresentError {
    match e {
        crate::fog::FogError::Invalid(reason) => PresentError::InvalidDraw {
            index: 0,
            reason: format!("rl9 {context}: {reason}"),
        },
        crate::fog::FogError::NoDevice(reason) => {
            PresentError::Device(format!("rl9 {context}: {reason}"))
        }
        crate::fog::FogError::Device(reason) => {
            PresentError::Device(format!("rl9 {context}: {reason}"))
        }
        crate::fog::FogError::Validation(reason) => {
            PresentError::Validation(format!("rl9 {context}: {reason}"))
        }
    }
}

// ---------------------------------------------------------------------------
// RL8: sky-forward sampling production wiring (one bake, three readers).
//
// Owner: render lane; consumer RA1 presenter (`encode_look_passes` plus
// `look_fog_params` on the window build, these headless fns everywhere).
// Units: linear HDR radiance triples, unit directions for the sampler.
// Determinism: the bake clones the day-capture texels and precomputes the
// ambient/fog means with f64 accumulation; the same capture gives
// bit-identical bytes twice (pinned in `sky.rs`). Errors: ragged captures
// fail as `PresentError` before any GPU work. Budget: one bake per capture
// change (the presenter caches it); the three readers allocate nothing.
// ---------------------------------------------------------------------------
//
// RA1 bottom-up note (BLOCKERS RA1 row, owned by RA1): the offscreen blit
// presents rows bottom-up (file row 0 holds view-bottom). That orientation
// lives in the blit UV mapping and is deliberately untouched here: the look
// bake samples in capture row order (row 0 at the zenith, like the day
// bake) and the fog pair rides the same blit UVs as the HDR target, so the
// skybox, IBL ambient and fog stay mutually consistent from one bake no
// matter how the RA1 owner resolves the blit orientation.

/// Map a sky failure into the presenter error (mirrors the `sky_err` seam
/// inside the GPU presenter so headless callers report the same shape).
pub fn rl8_err(context: &str, e: crate::sky::SkyError) -> PresentError {
    match e {
        crate::sky::SkyError::Invalid(reason) => PresentError::InvalidDraw {
            index: 0,
            reason: format!("rl8 {context}: {reason}"),
        },
        crate::sky::SkyError::NoDevice(reason) => {
            PresentError::Device(format!("rl8 {context}: {reason}"))
        }
        crate::sky::SkyError::Device(reason) => {
            PresentError::Device(format!("rl8 {context}: {reason}"))
        }
        crate::sky::SkyError::Validation(reason) => {
            PresentError::Validation(format!("rl8 {context}: {reason}"))
        }
    }
}

/// The one RL8 bake over the bound RL4 day capture: the texels the RA1
/// skybox samples plus the precomputed RL2 ambient and RL7 fog from the
/// same backing. Production caller (RA1 presenter); fails loudly on ragged
/// or non-finite input.
pub fn look_bake_for_capture(
    capture: &crate::sky::SkyCapture,
) -> Result<crate::sky::SkyLookBake, PresentError> {
    crate::sky::bake_look_from_day(capture)
        .map(|(bake, _)| bake)
        .map_err(|e| rl8_err("bake", e))
}

/// Reader 1 of 3 (RA1 skybox): bilinear lat-long sample of the bound bake
/// along `dir`. CPU twin of the `FORWARD_SKYBOX_WGSL` `skybox_sample` chunk;
/// the window skybox bind samples the same bake on the device.
pub fn skybox_sample_for_capture(
    capture: &crate::sky::SkyCapture,
    dir: [f32; 3],
) -> Result<[f32; 3], PresentError> {
    let bake = look_bake_for_capture(capture)?;
    crate::sky::sample_look_skybox(&bake, dir).map_err(|e| rl8_err("skybox", e))
}

/// Reader 2 of 3 (RL2 IBL ambient): mean radiance from the same bake the
/// RA1 skybox samples, so the ambient can never drift from the backdrop.
/// Non-finite or negative means fail loudly instead of tinting the frame.
pub fn ibl_ambient_for_capture(capture: &crate::sky::SkyCapture) -> Result<[f32; 3], PresentError> {
    let bake = look_bake_for_capture(capture)?;
    let ambient = crate::sky::look_ibl_ambient(&bake);
    if !ambient.iter().all(|c| c.is_finite()) || ambient.iter().any(|c| *c < 0.0) {
        return Err(PresentError::InvalidDraw {
            index: 0,
            reason: format!("rl8 ambient {ambient:?} is not finite nonnegative"),
        });
    }
    Ok(ambient)
}

/// Reader 3 of 3 (RL7 fog): horizon-ring color from the same bake the RA1
/// skybox samples, with the RL9 day extinction. Reads the precomputed fog
/// triple off the bake (no allocation); fails loudly on bad extinction.
pub fn fog_params_for_look_bake(
    bake: &crate::sky::SkyLookBake,
) -> Result<crate::fog::FogParams, PresentError> {
    let params = crate::fog::FogParams {
        fog_color: crate::sky::look_fog_color(bake),
        density_per_m: RL9_FOG_DENSITY_PER_M,
        falloff_m: RL9_FOG_FALLOFF_M,
        base_height_m: RL9_FOG_BASE_M,
        max_distance_m: RL9_FOG_MAX_DIST_M,
        exposure_ev100: RL9_FOG_EXPOSURE_EV100,
    };
    params.validate().map_err(|e| fog_err("look", e))?;
    Ok(params)
}

/// First-frame-hitch guard: run the one bake on the plan worker thread
/// (`render-worker`) and join before bind. The bake itself runs off the
/// presenting thread, so the first frame never waits on a same-thread bake;
/// the handle yields the same bake `look_bake_for_capture` computes.
pub fn bake_look_on_worker(
    capture: crate::sky::SkyCapture,
) -> Result<(crate::sky::SkyLookBake, f64), PresentError> {
    // A browser tab has no second thread: the same bake runs on the calling thread, same bytes.
    #[cfg(target_arch = "wasm32")]
    {
        crate::sky::bake_look_from_day(&capture).map_err(|e| rl8_err("bake", e))
    }
    #[cfg(not(target_arch = "wasm32"))]
    bake_look_on_thread(capture)
}

/// The worker-thread bake behind [`bake_look_on_worker`] on native targets.
#[cfg(not(target_arch = "wasm32"))]
fn bake_look_on_thread(
    capture: crate::sky::SkyCapture,
) -> Result<(crate::sky::SkyLookBake, f64), PresentError> {
    let handle = crate::sky::spawn_look_bake(capture).map_err(|e| rl8_err("bake", e))?;
    handle
        .join()
        .map_err(|_| PresentError::Device("rl8 look bake worker panicked".into()))?
        .map_err(|e| rl8_err("bake", e))
}

/// Build the RL9 fog params by ramping the fog color from the bound RL4 day
/// capture (sky, IBL ambient and fog can never drift); extinction mirrors
/// the RL7 day default. Fails loudly on a ragged capture.
///
/// RL8: runs the one look bake first and ramps from its RL7 reader, so this
/// is the same bake the RA1 skybox samples and the RL2 ambient reads. The
/// ramp is identical to the direct `from_sky_capture` path (the bake
/// precomputes `fog_color` from the same backing).
pub fn fog_params_for_capture(
    capture: &crate::sky::SkyCapture,
) -> Result<crate::fog::FogParams, PresentError> {
    let bake = look_bake_for_capture(capture)?;
    fog_params_for_look_bake(&bake)
}

/// Build the RL9 contact config for one live frame: the RL7 small-detail
/// kernel over the swap size. Zero-area (suspended) and over-bound frames
/// have no feed and fail loudly; the caller renders those without fog.
pub fn contact_for_frame(
    width_px: u32,
    height_px: u32,
) -> Result<crate::fog::ContactConfig, PresentError> {
    if SizeClass::classify(width_px, height_px) != SizeClass::Sizes {
        return Err(PresentError::InvalidSize {
            width: width_px,
            height: height_px,
        });
    }
    let contact = crate::fog::ContactConfig {
        width_px,
        height_px,
        radius_px: RL9_CONTACT_RADIUS_PX,
        thickness_m: RL9_CONTACT_THICKNESS_M,
        intensity: RL9_CONTACT_INTENSITY,
    };
    contact.validate().map_err(|e| fog_err("contact", e))?;
    Ok(contact)
}

/// True while the fog feed fits the frame: live look size plus both fog
/// transients inside [`crate::fog::FOG_TRANSIENT_BUDGET_BYTES`]. Oversize
/// frames (2048x2048 needs ~32 MiB over the 20 MiB budget) render without
/// fog instead of failing the frame.
pub fn fog_fits_frame(width_px: u32, height_px: u32) -> bool {
    if width_px == 0 || height_px == 0 {
        return false;
    }
    let bound = crate::gtao::MAX_LOOK_DIM;
    if width_px > bound || height_px > bound {
        return false;
    }
    let texels = width_px as u64 * height_px as u64;
    texels
        .saturating_mul(crate::fog::FOG_BYTES_PER_TEXEL)
        .saturating_mul(2)
        <= crate::fog::FOG_TRANSIENT_BUDGET_BYTES
}

/// Declare the RL9 fog plus contact plus forward wiring for one frame: the
/// production call site over [`crate::fog::compile_fog_for_forward`]. Pure
/// data (RA2 owns order and aliasing); fails loudly on bad configs.
pub fn compile_fog_for_presenter(
    params: &crate::fog::FogParams,
    contact: &crate::fog::ContactConfig,
) -> Result<crate::graph::CompiledGraph, PresentError> {
    crate::fog::compile_fog_for_forward(params, contact).map_err(|e| fog_err("graph", e))
}

// ---------------------------------------------------------------------------
// RL5: baked-GI bind consumption on the RA1 side (CPU receipt only).
//
// Owner: render lane; consumer WIN1/ED1 arena frame plus Q1 captures.
// The RA1 presenter bakes the canonical arena fixture on the CPU and files
// the validated [`crate::baked_gi::GiReceipt`] under the GI pass. No GPU
// texture is bound here: the device bind group lands orchestrator-side by
// reading [`crate::baked_gi::ArenaBake::to_bytes`]. Blit orientation is
// untouched (BLOCKERS RA1 row stays with the RA1 owner).
// ---------------------------------------------------------------------------

/// Map a baked-GI failure into the presenter error (mirrors the sky/fog
/// seams above the GPU presenter so headless callers report the same shape).
fn gi_err(context: &str, e: crate::baked_gi::BakedGiError) -> PresentError {
    match e {
        crate::baked_gi::BakedGiError::Invalid(reason) => PresentError::InvalidDraw {
            index: 0,
            reason: format!("rl5 {context}: {reason}"),
        },
    }
}

/// Bind the canonical baked arena for one RA1 frame: CPU bake plus validated
/// [`crate::baked_gi::bind_gi`] receipt. The receipt carries exact resident
/// bytes, the lightmap/probe counts and zero validation errors on the green
/// path; ragged viewports, exposures and sun directions fail loudly.
pub fn baked_gi_receipt_for_frame(
    frame_index: u64,
    viewport_px: [u32; 2],
    exposure_ev100: f32,
    sun_dir: [f32; 3],
) -> Result<crate::baked_gi::GiReceipt, PresentError> {
    let (bake, ms) = crate::baked_bake::bake_arena_fixture().map_err(|e| gi_err("bake", e))?;
    crate::baked_gi::bind_gi(frame_index, viewport_px, exposure_ev100, sun_dir, &bake, ms)
        .map_err(|e| gi_err("bind", e))
}

/// Sample one arena frame point through the canonical bake: lightmap texel
/// at `(u, v)` plus fighter irradiance at `pos` for `normal`. The call WIN1
/// makes per static fragment and per fighter each frame; steady-state frames
/// allocate nothing beyond the one bake. Invalid normals fail loudly.
pub fn baked_gi_frame_sample(
    bake: &crate::baked_gi::ArenaBake,
    u: f32,
    v: f32,
    pos: [f32; 3],
    normal: [f32; 3],
) -> Result<crate::baked_gi::GiFrameSample, PresentError> {
    crate::baked_gi::sample_arena_frame(bake, u, v, pos, normal).map_err(|e| gi_err("sample", e))
}

/// Pack the fog-composite uniform: the fog color as linear radiance (the
/// ramp itself: the blit mixes it into the linear HDR frame before the one
/// tone map) plus the frame size as floats for the blit texel lookup. Pure
/// bytes; validation stays on the caller.
pub fn fog_blit_uniform_bytes(color: [f32; 3], width_px: u32, height_px: u32) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[0..4].copy_from_slice(&color[0].to_le_bytes());
    out[4..8].copy_from_slice(&color[1].to_le_bytes());
    out[8..12].copy_from_slice(&color[2].to_le_bytes());
    out[12..16].copy_from_slice(&(width_px as f32).to_le_bytes());
    out[16..20].copy_from_slice(&(height_px as f32).to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fog_fits_frame_zero_area_has_no_feed() {
        assert!(!fog_fits_frame(0, 720));
        assert!(!fog_fits_frame(1280, 0));
        assert!(!fog_fits_frame(0, 0));
    }

    #[test]
    fn fog_fits_frame_hd_fits_budget() {
        assert!(fog_fits_frame(1280, 720));
    }

    #[test]
    fn fog_fits_frame_over_dim_has_no_feed() {
        assert!(!fog_fits_frame(4096, 720));
        assert!(!fog_fits_frame(1280, 4096));
    }

    #[test]
    fn fog_fits_frame_full_2048_square_over_budget() {
        // Doc promise: 2048x2048 needs ~32 MiB over the 20 MiB budget.
        assert!(!fog_fits_frame(2048, 2048));
    }

    #[test]
    fn fog_blit_uniform_round_trips_color_and_size() {
        let color = [0.25, 0.5, 1.0];
        let bytes = fog_blit_uniform_bytes(color, 1280, 720);
        assert_eq!(bytes.len(), FOG_BLIT_UNIFORM_BYTES);
        let r = f32::from_le_bytes(bytes[0..4].try_into().expect("r bytes"));
        let g = f32::from_le_bytes(bytes[4..8].try_into().expect("g bytes"));
        let b = f32::from_le_bytes(bytes[8..12].try_into().expect("b bytes"));
        let w = f32::from_le_bytes(bytes[12..16].try_into().expect("w bytes"));
        let h = f32::from_le_bytes(bytes[16..20].try_into().expect("h bytes"));
        assert_eq!((r, g, b), (color[0], color[1], color[2]));
        assert_eq!((w, h), (1280.0, 720.0));
        assert!(bytes[20..32].iter().all(|x| *x == 0));
    }

    #[test]
    fn fog_blit_uniform_distinguishes_colors() {
        let a = fog_blit_uniform_bytes([1.0, 0.0, 0.0], 1280, 720);
        let b = fog_blit_uniform_bytes([0.0, 1.0, 0.0], 1280, 720);
        assert_ne!(a, b);
    }

    #[test]
    fn contact_for_frame_hd_carries_rl9_kernel() {
        let contact = contact_for_frame(1280, 720).expect("hd frame builds contact");
        assert_eq!(contact.width_px, 1280);
        assert_eq!(contact.height_px, 720);
        assert_eq!(contact.radius_px, RL9_CONTACT_RADIUS_PX);
        assert_eq!(contact.thickness_m, RL9_CONTACT_THICKNESS_M);
        assert_eq!(contact.intensity, RL9_CONTACT_INTENSITY);
    }

    #[test]
    fn contact_for_frame_suspended_size_fails() {
        assert!(contact_for_frame(0, 720).is_err());
        assert!(contact_for_frame(1280, 0).is_err());
    }

    #[test]
    fn contact_for_frame_over_bound_fails() {
        assert!(contact_for_frame(4096, 720).is_err());
    }

    #[test]
    fn fog_err_invalid_carries_rl9_context() {
        let mapped = fog_err("contact", crate::fog::FogError::Invalid("bad".into()));
        match mapped {
            crate::present::surface::PresentError::InvalidDraw { reason, .. } => {
                assert!(reason.contains("rl9 contact"));
            }
            other => panic!("wrong mapping: {other:?}"),
        }
    }
}
