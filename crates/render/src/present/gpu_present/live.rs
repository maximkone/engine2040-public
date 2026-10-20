//! live: the window-path frames (live, opt-in probe, offscreen twin sink).
//!
//! Continues `impl Presenter` from `submit`. Owner: render lane, consumer the
//! windowed arena loop. Units: pixels for sizes, microseconds for waits.
//! The live frame is one command buffer and one submit with no readback;
//! the opt-in probe frame keeps the old blocking tranche (Game Engine Gems
//! 1 ch 9.6, 11.3: never wait on the GPU inside a frame).
use super::super::*;
use super::frame::{FrameReceipt, LookPassReceipt};
use super::gate::{ibl_cut_word, LiveLook, IBL_CUT_OFFSET};
use super::presenter::Presenter;
use super::submit::{
    effective_camera, inverse_view_proj_cols, map_readback_f32, padded_depth_row, ForwardLook,
    FrameCamera, MATERIAL_STRIDE,
};
use super::timing::PassesTimed;

/// Fog tranche rows in receipt order: fog passes, fog validation errors,
/// fog submit us, gtao passes, gtao validation errors, gtao submit us.
pub(super) type FogRows = (u32, u32, u64, u32, u32, u64);

/// The frame time the live meter assumes: the nominal 60 frames a second.
/// The meter never reads a wall clock, so a replay and a capture adapt
/// the same way. Units seconds.
pub(crate) const AUTO_LIVE_DT_S: f32 = 1.0 / 60.0;

/// Where a window-path frame blit lands.
#[derive(Clone, Copy)]
pub(super) enum LiveSink<'a> {
    /// The acquired swapchain view.
    Surface(&'a wgpu::TextureView),
    /// The LDR twin target; the live path never reads it back.
    Twin,
}

/// Whole microseconds since `started` (a reading of `engine_clock::now`),
/// saturating. Wall clock, evidence only, never hashed. The engine clock
/// rather than `std::time::Instant`: a browser tab has no std clock.
fn micros_since(started: std::time::Duration) -> u64 {
    u64::try_from(engine_clock::since(started).as_micros()).unwrap_or(u64::MAX)
}

/// Live half-res present switch: off keeps full-res pixels (default),
/// on renders the forward pass at half size and upscales in the blit.
/// Owner live, consumer `present_frame_live` below. Units pixels in/out.
/// Determinism pure size math plus one relaxed atomic load, no wall clock,
/// no allocation. Presentation only, never hashed.
static LIVE_HALF_RES: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Read the live half-res switch. Units boolean out.
#[must_use]
pub(super) fn live_half_res_enabled() -> bool {
    LIVE_HALF_RES.load(core::sync::atomic::Ordering::Relaxed)
}

/// Forward size for the live frame: full when off, halved (ceil) when on.
/// Pure, no allocation. Zero area passes through for the suspend path.
#[must_use]
pub(super) fn live_present_size(width: u32, height: u32, half: bool) -> (u32, u32) {
    if !half {
        return (width, height);
    }
    if width == 0 || height == 0 {
        return (width, height);
    }
    (width.div_ceil(2).max(1), height.div_ceil(2).max(1))
}

/// Live bloom gate: whether a halo can form for one frame's max luma.
/// Pure delegate to the post oracle [`crate::post::bloom_should_run`] with
/// no reimplemented foot, so the live frame can never drift from the oracle.
/// Owner live, consumer the future live bloom pass. Units linear HDR luma in,
/// boolean out. Determinism pure, no allocation, no device, no readback.
/// Fail-closed by delegation. Presentation only, never hashed.
#[must_use]
pub(super) fn live_bloom_should_run(max_luma: f32, threshold: f32, knee: f32) -> bool {
    crate::post::bloom_should_run(max_luma, threshold, knee)
}

/// Shipped live bloom default, mirroring the presenter's `BloomState::on()`
/// default: the window frame runs the halo without any caller raising the
/// switch, so `LF4`'s bloom is live in the picture the window ships.
/// Owner live, consumer `LIVE_BLOOM_ENABLED` below and the shipped-default
/// test. Units boolean. Presentation only, never hashed.
pub(super) const LIVE_BLOOM_DEFAULT_ON: bool = true;

/// Process-wide mirror of the bloom switch: ships at
/// [`LIVE_BLOOM_DEFAULT_ON`] and follows `Presenter::set_bloom_enabled`.
/// The live frame gates on its own presenter's `bloom.enabled`; this mirror
/// only reads back what the last call set. Owner live. Units boolean out.
/// Determinism one relaxed atomic. Presentation only, never hashed.
static LIVE_BLOOM_ENABLED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(LIVE_BLOOM_DEFAULT_ON);

/// Read the process-wide live bloom switch mirror that
/// `Presenter::set_bloom_enabled` writes. The live frame reads its own
/// presenter's switch instead; this mirror is what the gate tests read back.
/// Units boolean out.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "read back by the bloom gate tests only")
)]
#[must_use]
pub(super) fn live_bloom_enabled() -> bool {
    LIVE_BLOOM_ENABLED.load(core::sync::atomic::Ordering::Relaxed)
}

/// Set the live bloom switch. Units boolean in.
pub(super) fn set_live_bloom_enabled(enabled: bool) {
    LIVE_BLOOM_ENABLED.store(enabled, core::sync::atomic::Ordering::Relaxed);
}

/// Live bloom decision for one frame: false when the switch is off,
/// otherwise the [`live_bloom_should_run`] verdict for the frame's max
/// luma. Pure, no allocation, no device, no readback. Fail-closed by
/// delegation: bad luma or threshold/knee never runs. Presentation only,
/// never hashed.
#[must_use]
pub(super) fn live_bloom_for_frame(
    enabled: bool,
    max_luma: f32,
    threshold: f32,
    knee: f32,
) -> bool {
    if !enabled {
        return false;
    }
    live_bloom_should_run(max_luma, threshold, knee)
}

/// Live bloom threshold in linear HDR, mirrors
/// [`crate::post::BloomConfig::default`] plus `blit.wgsl`
/// `BLIT_BLOOM_THRESHOLD`. Owner live, consumer `present_frame_live`
/// below. Units linear HDR luma. Presentation only, never hashed.
pub(super) const LIVE_BLOOM_THRESHOLD: f32 = 1.0;

/// Live bloom knee fraction in `0..=1`, mirrors `BloomConfig::default`
/// plus the blit soft-knee divisor. Owner live, consumer as above.
/// Units fraction. Presentation only, never hashed.
pub(super) const LIVE_BLOOM_KNEE: f32 = 0.5;

/// Byte offset of the bloom enable inside the blit fog uniform
/// (`FogU::_pad0`). Owner live, consumer `present_frame_live` below.
/// Units bytes. The neutral uniform the targets carry is zero there,
/// so writing `0.0` keeps today's frame bit-identical.
const LIVE_BLOOM_FLAG_OFFSET: u64 = 20;

/// Byte offset of the camera exposure inside the blit fog uniform
/// (`FogU::exposure`). Owner live, consumer `present_frame_live` below.
/// Units bytes. The neutral uniform the targets carry holds 1.0 there, so a
/// meter that is off (or has not landed its first histogram) leaves the
/// frame exactly as it was.
const LIVE_EXPOSURE_OFFSET: u64 = 24;

/// Live bloom max-luma feed for one frame: CPU mirror of the forward HDR
/// peak from the draw list (brightest base-color channel times exposure
/// times light radiance), `None` when the feed is absent or bad so the
/// caller skips closed. Empty lists have no forward HDR feed and stay
/// skip; non-finite material, light or exposure stays skip. Pure, no
/// allocation, no device, no readback. Fail-closed: `None` maps to halo
/// off. Presentation only, never hashed.
/// Caller: `present_frame_live` below after `encode_forward`.
#[must_use]
pub(super) fn live_bloom_max_luma(list: &DrawList) -> Option<f32> {
    if list.items.is_empty() {
        return None;
    }
    let radiance = list.light.radiance().ok()?;
    let light_peak = radiance[0].max(radiance[1]).max(radiance[2]);
    if !light_peak.is_finite() || light_peak < 0.0 {
        return None;
    }
    let mut peak = 0.0f32;
    for item in &list.items {
        let base = item.material.base_color;
        if !base.iter().all(|c| c.is_finite()) {
            return None;
        }
        let exposure = item.material.exposure;
        if !exposure.is_finite() || exposure <= 0.0 {
            return None;
        }
        let base_peak = base[0].max(base[1]).max(base[2]);
        if !base_peak.is_finite() {
            return None;
        }
        let est = base_peak * exposure * light_peak;
        if !est.is_finite() {
            return None;
        }
        if est > peak {
            peak = est;
        }
    }
    Some(peak)
}

/// Live AO bind gate: true only when the depth plus normal feeds cover
/// the frame, so the caller keeps `ao_tex` at 1.0 on skip. Units: pixels
/// for sizes, texels for lens. Pure, no allocation. Presentation only,
/// never hashed. Anchor for the next wiring slice before
/// `live_ao_upload_for_frame`.
#[must_use]
pub(crate) fn live_ao_should_bind(
    width: u32,
    height: u32,
    depth_len: usize,
    normals_len: usize,
) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    let want = u64::from(width) * u64::from(height);
    let depth = match u64::try_from(depth_len) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let normals = match u64::try_from(normals_len) {
        Ok(v) => v,
        Err(_) => return false,
    };
    depth == want && normals == want
}

/// Live IBL gate for one frame: bound plus lit samples, else skip.
///
/// Owner live, consumer `present_frame_live` below. Units ms in, pixels
/// for sizes, microseconds out as `(passes, errors, submit_us)`. Pure with
/// no IO, clock, allocation or device; safe on any thread. Determinism the
/// same inputs give the same tuple on every run. Fail-closed: unbound or
/// unlit stays skipped at `(0, 0, 0)` even with bright ms, so the per-pass
/// table reads above zero only when the forward binds the bake and the
/// bake carries light.
#[must_use]
pub(crate) fn live_ibl_for_frame(
    forward_binds_ibl: bool,
    bake_lit: bool,
    ibl_ms: f64,
    w: u32,
    h: u32,
) -> (u32, u32, u64) {
    if !crate::ibl::live_ibl_should_sample(forward_binds_ibl, bake_lit) {
        return (0, 0, 0);
    }
    crate::ibl::live_ibl_receipt(ibl_ms, w, h)
}

/// Live IBL count gate for one frame: count only when bound, lit, timed and moved.
///
/// Owner live, consumer `present_frame_live` receipt below. Units ms in,
/// linear RGB gap in, boolean out. Pure, no allocation, no device.
/// Determinism same inputs give same verdict. Fail-closed: unbound or unlit
/// stays skip via [`crate::ibl::live_ibl_should_sample`]; zero time or
/// at-or-below 0.002 gap stays skip, mirroring the armor/sand epsilon.
/// Presentation only, never hashed.
/// Caller: `present_frame_live` receipt (timing lands next; this slice names the rule only).
#[must_use]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the receipt caller lands next; tests pin the rule"
    )
)]
pub(crate) fn live_ibl_should_count(forward_binds: bool, lit: bool, ibl_ms: f64, gap: f32) -> bool {
    if !crate::ibl::live_ibl_should_sample(forward_binds, lit) {
        return false;
    }
    if ibl_ms.is_nan() || ibl_ms <= 0.0 {
        return false;
    }
    if gap.is_nan() || gap <= 0.002 {
        return false;
    }
    true
}

/// Live sky gate for one frame: count only when bound and moved.
///
/// Owner live, consumer `present_frame_live` receipt below. Units ms in,
/// linear HDR gap in, boolean out. Pure, no allocation, no device.
/// Determinism same inputs give same verdict. Fail-closed: zero time or
/// at-or-below EG8 epsilon stays skip. Presentation only, never hashed.
#[must_use]
pub(crate) fn live_sky_should_count(sky_ms: f64, gap: f32) -> bool {
    if sky_ms.is_nan() || sky_ms <= 0.0 {
        return false;
    }
    if gap.is_nan() || gap <= 0.01 {
        return false;
    }
    true
}

/// Live sky draw gate for one frame: true while the sky row stays on, false
/// when the sky row lists its submit entry as skipped.
///
/// Owner live, consumer `present_frame_live` below. Units boolean out. Pure
/// with no IO, clock, allocation or device. Determinism same toggles give the
/// same bool twice. Fail-open to draw when the registry loses the sky row, so
/// enabled frames stay bit-identical to today. Presentation only, never hashed.
/// Caller: `present_frame_live` below (sky bind skip).
#[must_use]
pub(crate) fn live_sky_should_draw(toggles: &crate::pass_truth::LookToggles) -> bool {
    let skips = crate::pass_truth::submit_skips_for(toggles);
    let Some(index) = crate::pass_truth::LOOK_PASSES
        .iter()
        .position(|spec| spec.row == "sky")
    else {
        return true;
    };
    skips[index].is_none()
}

/// Live sky feed: horizon plus exposure from the bake, gated by sky switch.
/// Owner live, consumer `present_frame_live`. Pure, no allocation; off or bad
/// returns `None` so the frame stays identical.
#[must_use]
pub(crate) fn live_sky_feed_for_frame(
    toggles: &crate::pass_truth::LookToggles,
    bake: &crate::sky::SkyLookBake,
    exposure_ev100: f32,
) -> Option<([f32; 3], f32)> {
    if !live_sky_should_draw(toggles) {
        return None;
    }
    crate::sky::sky_live_feed(bake, exposure_ev100).ok()
}

/// FC3 30 fps budget gate: true when p50 sits at or below 33.3 ms and p99 at or below 50.0 ms.
///
/// Owner live, consumer the quiet perf gate (future perf-csv reader). Units ms in, boolean out.
/// Pure, no allocation, no device. Determinism same inputs give same bool.
/// Fail-closed: non-finite inputs refuse. Presentation only, never hashed.
/// Caller: future quiet-perf gate reading the windowed arena run.
#[must_use]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the perf gate reader lands later; tests pin the rule"
    )
)]
pub(crate) fn frame_meets_30fps_gate(p50_ms: f64, p99_ms: f64) -> bool {
    p50_ms <= 33.3 && p99_ms <= 50.0
}

impl Presenter {
    /// Stale AO reuse upload for the live frame: when the last probe
    /// tranche field still matches `width` by `height` and holds finite
    /// `0..=1` occlusion, upload it once into `ao_texture` for the blit
    /// slot with the cold-path layout and return the upload wall time in
    /// microseconds; empty or mismatched cache returns 0 with no extra work
    /// so the caller keeps `ao_tex` at 1.0. Units pixels for sizes,
    /// microseconds out. No map, no poll, no new bind group.
    /// Caller: `present_frame_live` below after `encode_forward`.
    fn stale_ao_reuse_upload(&self, width: u32, height: u32) -> u64 {
        let cached_ok = {
            let cached = self.gtao_sub.full_ao();
            live_ao_should_bind(width, height, cached.len(), cached.len())
                && crate::gtao::stale_ao_should_reuse(cached, width, height)
        };
        if !cached_ok {
            return 0;
        }
        let started = engine_clock::now();
        let wrote = {
            let cached = self.gtao_sub.full_ao();
            self.write_ao_full(width, height, cached)
        };
        if wrote.is_err() {
            return 0;
        }
        let ms = engine_clock::since(started).as_secs_f64() * 1000.0;
        Self::ms_to_us(ms)
    }

    /// Full-res AO upload for the live frame: validate `0..=1` field then
    /// write it once into `ao_texture` for the blit slot. Units pixels in,
    /// typed error out. Presentation only, never hashed. Owner live,
    /// consumer `stale_ao_reuse_upload` plus the fresh path below.
    fn write_ao_full(&self, width: u32, height: u32, full: &[f32]) -> Result<(), PresentError> {
        crate::gtao::check_ao_full_for_blit(full, width, height)
            .map_err(|e| PresentError::Validation(format!("ao full: {e}")))?;
        let targets = self
            .targets
            .as_ref()
            .ok_or_else(|| PresentError::Validation("live ao has no targets".into()))?;
        let w = usize::try_from(width).unwrap_or(0);
        let h = usize::try_from(height).unwrap_or(0);
        if w == 0 || h == 0 {
            return Err(PresentError::Validation("live ao frame is zero".into()));
        }
        let mut bytes = Vec::with_capacity(full.len().saturating_mul(4));
        for v in full {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let layout = wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width.saturating_mul(4)),
            rows_per_image: Some(height),
        };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &targets.ao_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            layout,
            size,
        );
        Ok(())
    }

    /// Fresh AO bake for one live frame: CPU oracle over the forward feed
    /// then guard plus upload. Units pixels plus meters, ms out. Empty feed
    /// stays skip via the caller guard; bad bake fails loudly. Presentation
    /// only, never hashed. Caller `present_frame_live` after forward.
    fn live_ao_upload_for_frame(
        &self,
        width: u32,
        height: u32,
        depth: &[f32],
        normals: &[[f32; 3]],
    ) -> Result<f64, PresentError> {
        if !live_ao_should_bind(width, height, depth.len(), normals.len()) {
            return Err(PresentError::Validation("live ao feed is empty".into()));
        }
        let config = Self::gtao_config_for_frame(width, height)?;
        let (full, ms) = crate::gtao::gtao_live_full_for_forward(depth, normals, &config)
            .map_err(|e| PresentError::Validation(format!("live ao bake: {e}")))?; // alloc-ok: cold bake-failure path, live guard skips empty feed
        crate::gtao::check_ao_full_for_blit(&full, width, height)
            .map_err(|e| PresentError::Validation(format!("live ao guard: {e}")))?; // alloc-ok: cold guard-failure path, never steady-state
        self.write_ao_full(width, height, &full)?;
        Ok(ms)
    }

    /// Identity draw order for one frame: `0..n` into `buf`, so the live
    /// path keeps list order without a scene graph. Units counts, no alloc.
    /// Presentation only, never hashed. Caller live paths below.
    fn frame_draw_order<'a>(
        list: &DrawList,
        buf: &'a mut [usize; MAX_DRAWS_PER_FRAME],
    ) -> &'a [usize] {
        let n = list.items.len().min(MAX_DRAWS_PER_FRAME);
        for (i, slot) in buf[..n].iter_mut().enumerate() {
            *slot = i;
        }
        &buf[..n]
    }

    /// Stage one frame from the shared camera plus the frame's built
    /// view-projection: same validation and uniform packing as `stage_frame`
    /// but reuses the caller-built camera and matrix instead of rebuilding
    /// either. Returns stack camera (80 B) plus light (32 B) images; every
    /// draw's material lands in `material_scratch`, so a staged frame
    /// allocates nothing. Units bytes out. Presentation only.
    /// Caller live paths below after `effective_camera`.
    fn stage_frame_with_camera(
        &mut self,
        list: &DrawList,
        camera: &crate::v1::Camera,
        view_proj: glam::Mat4,
        order: &[usize],
    ) -> Result<([u8; 80], [u8; 32]), PresentError> {
        list.validate()?;
        if order.len() != list.items.len() {
            return Err(PresentError::InvalidDraw {
                index: 0,
                reason: "draw order length mismatches list".into(),
            });
        }
        list.light
            .validate()
            .map_err(|e| PresentError::InvalidDraw {
                index: 0,
                reason: format!("light: {e}"),
            })?;
        let (cam_u, light_u, _) = crate::v1::build_uniforms_with_view_proj(
            camera,
            view_proj,
            &list.light,
            &crate::v1::PbrMaterial {
                base_color: [0.5, 0.5, 0.5],
                metallic: 0.0,
                roughness: 0.5,
                exposure: 1.0,
            },
        )
        .map_err(|e| PresentError::InvalidDraw {
            index: 0,
            reason: format!("frame: {e}"),
        })?;
        self.material_scratch.clear();
        for (slot, draw) in order.iter().enumerate() {
            let item = list
                .items
                .get(*draw)
                .ok_or_else(|| PresentError::InvalidDraw {
                    index: slot,
                    reason: "draw order escapes list".into(),
                })?;
            // Material-only pack: the camera plus light uniforms above already
            // carry this frame's matrix, so no per-draw view math runs here.
            let mat_u = crate::v1::build_material(&item.material).map_err(|e| {
                PresentError::InvalidDraw {
                    index: slot,
                    reason: format!("material: {e}"),
                }
            })?;
            if self.meshes.get(&item.mesh).is_none() {
                return Err(PresentError::InvalidDraw {
                    index: slot,
                    reason: format!("unknown mesh key {:?}", item.mesh.id),
                });
            }
            // Texture residency: unknown keys in any named slot fail with
            // their draw index, mirroring meshes, before any GPU work.
            self.require_draw_textures(item, slot)?;
            // Stack image, not `to_bytes()`: the per-draw byte image would
            // build one `Vec` per draw, which the ALLOC ratchet cannot see.
            self.material_scratch.extend_from_slice(&mat_u.to_array());
            self.material_scratch
                .resize((slot + 1) * MATERIAL_STRIDE, 0);
        }
        Ok((cam_u.to_array(), light_u.to_array()))
    }

    /// The auto-exposure meter step of one frame: the histogram of the
    /// frame before this one lands here (the readback is one frame late),
    /// the state adapts toward the key, and the stops ride the next
    /// frame's draws. A frame whose map has not landed keeps the stops it
    /// had; the meter never guesses.
    pub(super) fn auto_exposure_meter_step(&mut self) -> Result<(), PresentError> {
        if !self.auto_exp_enabled {
            return Ok(());
        }
        let Some(hist) = self.auto_exp_gpu.take_ready()? else {
            return Ok(());
        };
        self.auto_exp_hist = Some(hist);
        self.auto_exp_state.tick_hist(&hist, AUTO_LIVE_DT_S)?;
        Ok(())
    }

    /// The linear multiplier of the adapted stops: 1.0 while the meter is
    /// off, so a frame without a meter is exactly the frame the caller
    /// staged.
    pub(crate) fn auto_exposure_mult(&self) -> f32 {
        if !self.auto_exp_enabled {
            return 1.0;
        }
        self.auto_exp_state.mult()
    }

    /// After-blit overlay draw for staged HP bars: one textured-quad draw
    /// over the blit view with no depth. Skips empty plans or verts so the
    /// live frame stays bit-identical when no HUD is staged. Units quads.
    /// Presentation only, never hashed. Callers `present_frame_live` and
    /// `submit::render_offscreen` (the capture twin).
    pub(super) fn encode_overlay(
        &self,
        enc: &mut wgpu::CommandEncoder,
        plan: Option<crate::overlay::OverlayDraw>,
        verts: &[crate::overlay::QuadVertex],
        mvp_bytes: &[u8],
        out_view: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
    ) {
        use wgpu::util::DeviceExt;
        let Some(draw) = plan else {
            return;
        };
        if draw.verts == 0 || verts.is_empty() || mvp_bytes.len() < 64 {
            return;
        }
        // One indexed draw for the whole ring: four verts per staged quad,
        // six indices per quad (two triangles). Empty stays no-op above so
        // the live frame is bit-identical with no HUD staged.
        let quads = draw.quads.min(verts.len() / crate::overlay::VERTS_PER_QUAD);
        if quads == 0 {
            return;
        }
        let vert_count = quads.saturating_mul(crate::overlay::VERTS_PER_QUAD);
        let used = verts.split_at(vert_count.min(verts.len())).0;
        let stride = core::mem::size_of::<crate::overlay::QuadVertex>();
        let mut bytes = Vec::with_capacity(used.len().saturating_mul(stride));
        for v in used {
            for c in v.pos2 {
                bytes.extend_from_slice(&c.to_le_bytes());
            }
            for c in v.uv2 {
                bytes.extend_from_slice(&c.to_le_bytes());
            }
            bytes.extend_from_slice(&v.col4);
        }
        if bytes.is_empty() {
            return;
        }
        let mut index: Vec<u32> = Vec::with_capacity(quads.saturating_mul(6));
        for (i, _) in used
            .chunks_exact(crate::overlay::VERTS_PER_QUAD)
            .enumerate()
        {
            let base =
                u32::try_from(i.saturating_mul(crate::overlay::VERTS_PER_QUAD)).unwrap_or(u32::MAX);
            if base == u32::MAX {
                return;
            }
            index.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 1, base + 3]);
        }
        if index.is_empty() {
            return;
        }
        let vb = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ra1-overlay-vb"),
                contents: &bytes,
                usage: wgpu::BufferUsages::VERTEX,
            });
        let mut index_bytes = Vec::with_capacity(index.len().saturating_mul(4));
        for word in &index {
            index_bytes.extend_from_slice(&word.to_le_bytes());
        }
        let ib = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ra1-overlay-ib"),
                contents: &index_bytes,
                usage: wgpu::BufferUsages::INDEX,
            });
        let ub = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ra1-overlay-mvp"),
                contents: mvp_bytes.split_at(64).0,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        // Bitmap atlas for this slice: one white texel, so solid HP quads
        // keep their exact color and glyph quads sample a real texture.
        // The full font raster lands in a later slice without moving this draw.
        let atlas = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ra1-overlay-atlas"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255u8, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let atlas_view = atlas.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ra1-overlay-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let bind_layout = pipeline.get_bind_group_layout(0);
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ra1-overlay-bind"),
            layout: &bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ub.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let index_len = u32::try_from(index.len()).unwrap_or(0);
        if index_len == 0 {
            return;
        }
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ra1-overlay"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: out_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(pipeline);
        rp.set_bind_group(0, &bind, &[]);
        rp.set_vertex_buffer(0, vb.slice(..));
        rp.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
        rp.draw_indexed(0..index_len, 0, 0..1);
    }
}

/// Revision over forward inputs (mesh, material, base-color, count).
/// Same list gives same revision; any id, digest, material or count
/// change moves it. Pure, no allocation. Owner live, consumer
/// `present_frame_live` (the bind reuse skip). Units u64 out.
#[must_use]
pub(super) fn live_forward_revision(list: &DrawList) -> u64 {
    let mut h = engine_core::Fnv64::new();
    h.write_u64(u64::try_from(list.items.len()).unwrap_or(u64::MAX));
    for item in &list.items {
        h.write_u64(item.mesh.id);
        h.write(&item.mesh.digest);
        for c in item.material.base_color {
            h.write_f32(c);
        }
        h.write_f32(item.material.metallic);
        h.write_f32(item.material.roughness);
        h.write_f32(item.material.exposure);
        match &item.base_color {
            Some(key) => {
                h.write_u64(key.id);
                h.write(&key.digest);
            }
            None => {
                h.write_u64(0x9e37_79b9_7f4a_7c15);
            }
        }
    }
    h.finish()
}

/// Live bind-reuse skip switch: off rebuilds forward binds every frame
/// (default), on reuses the last `draw_binds` when the draw-list revision
/// repeats. Owner live, consumer `present_frame_live` below. Units boolean.
/// Determinism pure revision compare, no wall clock, no allocation.
/// Presentation only, never hashed.
static LIVE_BIND_SKIP_ENABLED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Last live bind revision; `None` until the first rebuilt frame.
/// Relaxed atomics only, per-frame loads.
static LIVE_BIND_LAST_HAS: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
static LIVE_BIND_LAST_VALUE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read the live bind-reuse skip switch. Units boolean out.
#[must_use]
pub(crate) fn live_bind_skip_enabled() -> bool {
    LIVE_BIND_SKIP_ENABLED.load(core::sync::atomic::Ordering::Relaxed)
}

/// Set the live bind-reuse skip switch. Units boolean in.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the skip ships off; only tests flip it")
)]
pub(crate) fn set_live_bind_skip_enabled(enabled: bool) {
    LIVE_BIND_SKIP_ENABLED.store(enabled, core::sync::atomic::Ordering::Relaxed);
}

/// Cached bind revision; `None` until the first rebuild.
fn live_bind_last_cached() -> Option<u64> {
    if LIVE_BIND_LAST_HAS.load(core::sync::atomic::Ordering::Relaxed) {
        Some(LIVE_BIND_LAST_VALUE.load(core::sync::atomic::Ordering::Relaxed))
    } else {
        None
    }
}

/// Remember the last rebuilt bind revision.
fn live_bind_last_store(current: u64) {
    LIVE_BIND_LAST_VALUE.store(current, core::sync::atomic::Ordering::Relaxed);
    LIVE_BIND_LAST_HAS.store(true, core::sync::atomic::Ordering::Relaxed);
}

/// Rebuild vs reuse for one revision; disabled always rebuilds. Pure.
#[must_use]
pub(crate) fn live_bind_should_skip(cached: Option<u64>, current: u64, enabled: bool) -> bool {
    if !enabled {
        return false;
    }
    match cached {
        Some(known) => known == current,
        None => false,
    }
}

/// Live present-path reuse for one frame: true only when the switch is on,
/// the draw-list revision repeats and the bound count still matches the
/// frame order, so a resized list rebuilds once instead of reusing stale
/// binds. Owner live, consumer `present_frame_live` below. Units counts in,
/// boolean out. Determinism pure compare, no wall clock, no allocation.
/// Presentation only, never hashed.
#[must_use]
pub(crate) fn live_present_should_reuse(
    cached: Option<u64>,
    current: u64,
    enabled: bool,
    binds_len: usize,
    order_len: usize,
) -> bool {
    if binds_len != order_len {
        return false;
    }
    live_bind_should_skip(cached, current, enabled)
}

/// After-blit overlay pipeline for one output format: textured quads,
/// alpha blend, no depth; layout follows the shared overlay helpers.
/// Owner live, consumer `present_frame_live` plus the capture path in
/// `submit::render_offscreen`.
pub(super) fn live_overlay_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    use crate::pipeline_warmup::CountedPipelines as _;
    let desc = crate::overlay::overlay_pipeline_desc();
    let layout = crate::overlay::overlay_vertex_layout();
    let stride = u64::try_from(layout.stride).unwrap_or(u64::MAX);
    let pos = u64::try_from(layout.pos_offset).unwrap_or(u64::MAX);
    let col = u64::try_from(layout.col_offset).unwrap_or(u64::MAX);
    let uv = pos.saturating_add(8);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ra1-overlay"),
        source: wgpu::ShaderSource::Wgsl(crate::overlay::OVERLAY_WGSL.into()),
    });
    let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ra1-overlay-bind-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ra1-overlay-layout"),
        bind_group_layouts: &[Some(&bind_layout)],
        immediate_size: 0,
    });
    let blend = if desc.alpha_blend {
        Some(wgpu::BlendState::ALPHA_BLENDING)
    } else {
        Some(wgpu::BlendState::REPLACE)
    };
    device.create_render_pipeline_counted(&wgpu::RenderPipelineDescriptor {
        label: Some("ra1-overlay-pipeline"),
        layout: Some(&pipe_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some(desc.vs_entry),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: stride,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: pos,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: uv,
                        shader_location: 1,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Unorm8x4,
                        offset: col,
                        shader_location: 2,
                    },
                ],
            })],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(desc.fs_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl Presenter {
    /// Refresh the per-frame uniforms: camera, light and every draw's
    /// material in one write to the ring (`stage_frame` packed them).
    /// Three buffer writes per frame, however many draws.
    pub(super) fn write_frame_uniforms(&self, cam: &[u8], light: &[u8], ibl_on: bool) {
        self.queue.write_buffer(&self.cam_buf, 0, cam);
        self.queue.write_buffer(&self.light_buf, 0, light);
        // The IBL cut word rides every frame, so a frame that cuts the baked
        // ambient never leaks the cut into the next frame.
        self.queue.write_buffer(
            &self.ibl_sh_buf,
            IBL_CUT_OFFSET,
            &ibl_cut_word(ibl_on).to_le_bytes(),
        );
        if !self.material_scratch.is_empty() {
            self.queue
                .write_buffer(&self.material_ring, 0, &self.material_scratch);
        }
    }

    /// End a frame: drain the buffers and bind groups created since the
    /// last frame ended into the frame tally (uploads the game made between
    /// frames count toward the frame that follows) and fold it into the
    /// totals for `kind`.
    pub(super) fn finish_frame(&mut self, kind: FrameKind) {
        let (buffers, bind_groups) = self.creates.take_frame();
        self.frame_waits.buffers = buffers;
        self.frame_waits.bind_groups = bind_groups;
        self.waits.fold(kind, self.frame_waits);
    }

    /// Refresh the fog uniform the forward fragment and the sky draw read:
    /// inverse view-projection and eye for the sky rays, the height-fog
    /// slope for both. Reads the frame camera plus its built matrix, so the
    /// frame never builds the matrix a second time. `shader_fog` false (the
    /// capture twin and the probe composite, which fog through the blit pair)
    /// writes a zero slope, so the forward mix is exactly 0.0 and nothing fogs
    /// twice. The kill switch (`set_fog_enabled`) zeroes the slope on the live
    /// frame too.
    pub(super) fn write_fog_forward(
        &self,
        frame: &FrameCamera,
        shader_fog: bool,
    ) -> Result<(), PresentError> {
        let inv = inverse_view_proj_cols(frame)?;
        let mut flat = [0.0f32; 16];
        for (dst, src) in flat.chunks_exact_mut(4).zip(inv.iter()) {
            dst.copy_from_slice(src);
        }
        let fog = if shader_fog && self.fog_enabled {
            self.fog_fwd
        } else {
            self.fog_fwd.off()
        };
        self.queue.write_buffer(
            &self.fog_fwd_buf,
            0,
            &fog.to_bytes(&flat, frame.eye.to_array()),
        );
        Ok(())
    }

    /// Frame receipt over the cache budgets plus this frame's pass rows.
    /// One writer for the capture twin, the probe frame and the live frame.
    pub(super) fn frame_receipt(
        &self,
        draws: usize,
        look: &LookPassReceipt,
        ibl: (u32, u32, u64),
        fog: FogRows,
    ) -> FrameReceipt {
        let (
            mesh_count,
            texture_count,
            mesh_high_water,
            texture_high_water,
            mesh_evicted,
            texture_evicted,
            evicted_mesh_id,
            evicted_texture_id,
        ) = self.cache_receipt_fields();
        FrameReceipt {
            width: self.swap.size().0,
            height: self.swap.size().1,
            draws: u32::try_from(draws).unwrap_or(u32::MAX),
            validation_errors: 0,
            generation: self.swap.generation(),
            presented: true,
            adapter: self.adapter_name.clone(),
            driver: self.adapter_driver.clone(),
            mesh_count,
            texture_count,
            mesh_high_water,
            texture_high_water,
            mesh_evicted,
            texture_evicted,
            evicted_mesh_id,
            evicted_texture_id,
            look_passes: look.passes,
            look_validation_errors: look.validation_errors,
            ibl_passes: ibl.0,
            ibl_validation_errors: ibl.1,
            fog_passes: fog.0,
            fog_validation_errors: fog.1,
            fog_submit_us: fog.2,
            gtao_passes: fog.3,
            gtao_validation_errors: fog.4,
            gtao_submit_us: fog.5,
            sky_submit_us: {
                let raw_us = Self::ms_to_us(look.sky_ms);
                let gap = crate::sky::sky_live_on_off_gap(&self.sky_look).unwrap_or(0.0);
                let sky_ms = std::time::Duration::from_micros(raw_us).as_secs_f64() * 1000.0;
                if live_sky_should_count(sky_ms, gap) {
                    raw_us
                } else {
                    0
                }
            },
            layer_submit_us: Self::ms_to_us(look.layer_ms),
            particle_submit_us: Self::ms_to_us(look.particle_ms),
            decal_submit_us: Self::ms_to_us(look.decal_ms),
            ibl_submit_us: ibl.2,
            shadow_submit_us: 0,
        }
    }

    /// Blit view and pipeline for a window-path sink. A missing surface
    /// pipeline or target set is a typed error, never a panic.
    fn sink_target<'a>(
        &'a self,
        sink: LiveSink<'a>,
    ) -> Result<(&'a wgpu::TextureView, &'a wgpu::RenderPipeline), PresentError> {
        match sink {
            LiveSink::Surface(view) => {
                let pipeline = self
                    .surface_pipeline
                    .as_ref()
                    .map(|(_, pipeline)| pipeline)
                    .ok_or(PresentError::NoSurface)?;
                Ok((view, pipeline))
            }
            LiveSink::Twin => {
                let targets = self
                    .targets
                    .as_ref()
                    .ok_or_else(|| PresentError::Validation("live twin has no targets".into()))?;
                Ok((&targets.out_view, &self.twin_pipeline))
            }
        }
    }

    /// Render one window-path frame into the LDR twin with no readback:
    /// the same encode the surface takes (live frame by default, probe
    /// frame under `set_look_probes`), minus the swapchain. For tests and
    /// headless timing runs that need the window frame's cost without a
    /// window. The pixels stay on the device.
    pub fn render_live_offscreen(&mut self, list: &DrawList) -> Result<FrameReceipt, PresentError> {
        self.render_live_offscreen_with_toggles(list, &crate::pass_truth::LookToggles::all_on())
    }

    /// Window-path frame with an explicit look-toggle set: sky disabled skips
    /// the sky bind in `present_frame_live`, all other passes behave as today.
    /// Owner live, consumer the coming capture harness, which renders each
    /// arena frame with one pass on and off. Units toggles in, receipt out.
    /// Presentation only, never hashed.
    /// Caller: `render_live_offscreen` above (all-on arm) plus the harness.
    pub fn render_live_offscreen_with_toggles(
        &mut self,
        list: &DrawList,
        toggles: &crate::pass_truth::LookToggles,
    ) -> Result<FrameReceipt, PresentError> {
        self.check_device()?;
        if self.suspended() {
            return Ok(self.suspended_receipt());
        }
        let (width, height) = self.swap.size();
        let receipt = self.window_path_frame(LiveSink::Twin, list, width, height, toggles)?;
        self.frames_presented += 1;
        Ok(receipt)
    }

    /// One window-path frame: the probe composite under the opt-in, the
    /// live frame otherwise.
    fn window_path_frame(
        &mut self,
        sink: LiveSink<'_>,
        list: &DrawList,
        width: u32,
        height: u32,
        toggles: &crate::pass_truth::LookToggles,
    ) -> Result<FrameReceipt, PresentError> {
        if self.look_probes {
            self.present_frame_probed(sink, list, width, height)
        } else {
            self.present_frame_live(sink, list, width, height, toggles)
        }
    }

    pub(super) fn present_frame(
        &mut self,
        frame: wgpu::SurfaceTexture,
        list: &DrawList,
        width: u32,
        height: u32,
    ) -> Result<FrameReceipt, PresentError> {
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let receipt = self.window_path_frame(
            LiveSink::Surface(&view),
            list,
            width,
            height,
            &crate::pass_truth::LookToggles::all_on(),
        )?;
        if let Some(format) = self.surface_config.as_ref().map(|config| config.format) {
            self.capture_after_frame(&frame.texture, format, width, height);
        }
        let present_started = engine_clock::now();
        self.queue.present(frame);
        super::frame::record_present_call_us(micros_since(present_started));
        self.frames_presented += 1;
        Ok(receipt)
    }

    /// The live window frame: one command buffer, one submit. Forward
    /// draws with the height fog evaluated per fragment, the sky at the far
    /// plane, then the blit. No depth readback, no CPU decode, no
    /// compute-pass readback, no map and no poll-wait: the CPU never waits
    /// on the GPU (Game Engine Gems 1 ch 9.6, 11.3). The timestamps this
    /// frame resolves are read a few frames later by a non-blocking map.
    fn present_frame_live(
        &mut self,
        sink: LiveSink<'_>,
        list: &DrawList,
        width: u32,
        height: u32,
        toggles: &crate::pass_truth::LookToggles,
    ) -> Result<FrameReceipt, PresentError> {
        #[cfg(target_arch = "wasm32")]
        self.scopes.settle();
        let stage_started = engine_clock::now();
        let (render_w, render_h) = live_present_size(width, height, live_half_res_enabled());
        let frame = effective_camera(list, render_w, render_h)?;
        let mut order_buf = [0usize; MAX_DRAWS_PER_FRAME];
        let order = Self::frame_draw_order(list, &mut order_buf);
        // The meter step runs before the frame is staged: the histogram
        // of the frame before this one lands here (the readback is one
        // frame late), and the stops it holds ride the blit exposure of
        // this frame.
        self.auto_exposure_meter_step()?;
        let (cam_bytes, light_bytes) =
            self.stage_frame_with_camera(list, &frame, frame.built_view_proj, order)?;
        self.frame_waits = FrameWaits::default();
        self.ensure_targets(render_w, render_h);
        let mut look = LiveLook::from_toggles(toggles);
        look.gtao = crate::quality::gtao_runs(&self.quality, look.gtao);
        self.write_frame_uniforms(&cam_bytes, &light_bytes, look.ibl);
        let gtao_run = self.gtao_live_plan(look.gtao, render_w, render_h);
        self.bind_twin_shadow(Self::twin_focus_depth(list))?;
        self.write_fog_forward(&frame, true)?;
        if self.neutral_fog_pending {
            // Leaving the probe composite: the pair still holds its last
            // fog and occlusion, reset it once.
            let _rows = self.clear_fog_targets(render_w, render_h);
            self.neutral_fog_pending = false;
        }
        // Caller: `present_frame_live` live present path. Behavior: when the
        // present skip is on and the draw-list revision repeats with a
        // matching bind count, reuse the last `draw_binds`; a length
        // mismatch still rebuilds once so no stale bind reaches the draw.
        let live_current = live_forward_revision(list);
        let live_cached = live_bind_last_cached();
        if !live_present_should_reuse(
            live_cached,
            live_current,
            live_bind_skip_enabled(),
            self.draw_binds.len(),
            order.len(),
        ) {
            self.resolve_draw_binds(list)?;
            live_bind_last_store(live_current);
        }
        let stage_us = micros_since(stage_started);
        let submit_started = engine_clock::now();
        let timer_slot = self
            .gpu_timer
            .as_mut()
            .and_then(|timer| timer.begin_frame(&self.device));
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ra1-live-frame"),
            });
        // EG8 sky row: consult the pure gate so the sky row disabled skips the
        // sky pass; all-on draws it (LF1: the live frame shows the sky bake).
        // Caller: `live_sky_should_draw` above over `submit_skips_for`.
        let sky_draw = live_sky_should_draw(toggles);
        let stale_us: u64;
        let gtao_timed: bool;
        // The `ibl` cell is the forward IBL-bound encode and nothing else, as
        // `ibl::live_ibl_receipt` documents it: the clock below opens on that
        // one pass and closes on it, so the AO upload, blit and overlay never
        // bill to the `ibl` column.
        let forward_ibl_ms: f64;
        // True when the meter dispatch of this frame went into the
        // encoder; the map of its bins starts after the submit. False
        // while the meter is off or a map is still open.
        let mut histogram_dispatched = false;
        {
            let targets = self
                .targets
                .as_ref()
                .ok_or_else(|| PresentError::Validation("live frame has no targets".into()))?;
            let (blit_view, blit_pipeline) = self.sink_target(sink)?;
            let timer = self.gpu_timer.as_ref().zip(timer_slot);
            // LF1: re-read bake pair before forward; `None` keeps off identical.
            if let Some((sky_horizon, sky_exposure)) = live_sky_feed_for_frame(
                toggles,
                &self.sky_look,
                crate::arena_day::ARENA_DAY_EXPOSURE_EV100,
            ) {
                debug_assert_eq!(sky_horizon, self.fog_fwd.color);
                debug_assert_eq!(sky_exposure, self.sky_exposure);
            }
            // LF3 landing 2: the depth prepass writes the frame's depth, the
            // GTAO kernel turns it into the occlusion field, and the forward
            // pass that follows scales its ambient diffuse by that field.
            // The forward pass then clears the depth and writes it again,
            // so the frame's final depth is the one it wrote before.
            gtao_timed = self.encode_ao_field(
                &mut enc,
                targets,
                list,
                &frame,
                // The field needs the geometry: a frame with no draws has
                // nothing to occlude, so the field stays open.
                gtao_run && look.forward,
                timer.map(|(timer, _)| timer.gtao_writes()),
            )?;
            let forward_ibl_started = engine_clock::now();
            // The particle batch is the probe tranche's own draw (QL12): a
            // live frame stages nothing, so it draws nothing and its pixels
            // stay a pure function of the frame's list.
            let sky_timed = self.encode_forward(
                &mut enc,
                targets,
                list,
                ForwardLook::live(&look, sky_draw, gtao_run, timer.map(|(timer, _)| timer)),
            );
            forward_ibl_ms = engine_clock::since(forward_ibl_started).as_secs_f64() * 1000.0;
            // LF3 first landing: the CPU GTAO helper
            // (`live_ao_upload_for_frame`: bake via
            // `gtao_live_full_for_forward`, guard via `check_ao_full_for_blit`,
            // upload via `write_ao_full`). The forward-stored depth plus
            // normals feed is absent (the device pass above owns the field
            // now), so the empty feed stays skip (ao_tex keeps 1.0) with the
            // stale reuse below as fallback. A guard failure records skip,
            // never fails the frame; ms is evidence only against
            // `GTAO_BUDGET_MS`.
            let depth: &[f32] = &[];
            let normals: &[[f32; 3]] = &[];
            let fresh_us = if live_ao_should_bind(render_w, render_h, depth.len(), normals.len()) {
                self.live_ao_upload_for_frame(render_w, render_h, depth, normals)
                    .map(Self::ms_to_us)
                    .unwrap_or(0)
            } else {
                0
            };
            let fallback_us = if fresh_us == 0 && look.gtao && !gtao_run {
                self.stale_ao_reuse_upload(render_w, render_h)
            } else {
                0
            };
            stale_us = fresh_us.max(fallback_us);
            // LF4 bloom gate: CPU mirror of the forward HDR peak feeds the
            // post oracle; the flag rides `FogU::_pad0` (0.0 off keeps
            // today's frame bit-identical, 1.0 on lets the blit halo lift).
            // Bad feed or switch off stays off. The switch is this
            // presenter's own, so two presenters never share one flag.
            // Caller `live_bloom_for_frame`.
            let bloom_on = look.bloom
                && live_bloom_max_luma(list).is_some_and(|peak| {
                    live_bloom_for_frame(
                        self.bloom.enabled,
                        peak,
                        LIVE_BLOOM_THRESHOLD,
                        LIVE_BLOOM_KNEE,
                    )
                });
            let bloom_flag: f32 = if bloom_on { 1.0 } else { 0.0 };
            self.queue.write_buffer(
                &targets.fog_col_buf,
                LIVE_BLOOM_FLAG_OFFSET,
                &bloom_flag.to_le_bytes(),
            );
            // GS44: the one camera exposure of the frame, applied by the
            // blit before the tone map. The meter is one frame late and
            // fail-closed, so this writes 1.0 until a histogram lands.
            self.queue.write_buffer(
                &targets.fog_col_buf,
                LIVE_EXPOSURE_OFFSET,
                &self.auto_exposure_mult().to_le_bytes(),
            );
            self.encode_blit(
                &mut enc,
                &targets.blit_bind,
                blit_view,
                blit_pipeline,
                timer.map(|(timer, _)| timer.blit_writes()),
            );
            let overlay_format = match sink {
                LiveSink::Twin => super::OUTPUT_FORMAT,
                LiveSink::Surface(_) => self
                    .surface_config
                    .as_ref()
                    .map(|c| c.format)
                    .unwrap_or(super::OUTPUT_FORMAT),
            };
            if let Some(plan) = self.overlay.hp_bars_plan(frame.aspect) {
                let mut verts = [crate::overlay::QuadVertex::default();
                    crate::overlay::OVERLAY_MAX_QUADS * crate::overlay::VERTS_PER_QUAD];
                let written = self.overlay.write_vertices(&mut verts);
                let count = plan.verts.min(written);
                if count > 0 {
                    let mut mvp_bytes = [0u8; 64];
                    let mut at = 0usize;
                    for col in plan.mvp.iter() {
                        for v in col.iter() {
                            let bytes = v.to_le_bytes();
                            mvp_bytes[at..at + 4].copy_from_slice(&bytes);
                            at = at.saturating_add(4);
                        }
                    }
                    let overlay_pipeline = self.overlay_pipeline_for(overlay_format);
                    self.encode_overlay(
                        &mut enc,
                        Some(plan),
                        &verts[..count],
                        &mvp_bytes,
                        blit_view,
                        &overlay_pipeline,
                    );
                }
            }
            if let Some((timer, slot)) = timer {
                let timed = PassesTimed {
                    sky: sky_timed,
                    gtao: gtao_timed,
                    particle: false,
                };
                timer.encode_resolve(&mut enc, slot, timed);
            }
        }
        // The shipped forward layout carries the bake binds and `fs` samples,
        // so the live frame is bound; the bake verdict comes from the live bake.
        let bake_lit = crate::ibl::bake_has_light(&self.ibl_bake);
        let ibl = live_ibl_for_frame(true, bake_lit, forward_ibl_ms, render_w, render_h);
        // The meter: one compute dispatch over the HDR target the forward
        // wrote, into the 64 log-luma bins. The blit applies the
        // exposure, so the target still holds pre-exposure radiance:
        // the meter reads the scene, not the exposed frame.
        if self.auto_exp_enabled {
            let hdr_view = self
                .targets
                .as_ref()
                .map(|targets| targets.hdr_view.clone());
            if let Some(hdr_view) = hdr_view {
                histogram_dispatched = self
                    .auto_exp_gpu
                    .encode(&mut enc, &hdr_view, render_w, render_h);
            }
        }
        self.queue.submit(Some(enc.finish()));
        if histogram_dispatched {
            // The map of the bins is picked up by the frame after this
            // one: the live frame never waits on the GPU.
            self.auto_exp_gpu.begin_map();
        }
        // The present call is patched in by `present_frame` on the surface
        // path; the twin never makes one.
        super::frame::record_live_wall_split(super::frame::LiveWallSplit {
            stage_us,
            submit_us: micros_since(submit_started),
            present_call_us: 0,
        });
        if let (Some(timer), Some(slot)) = (self.gpu_timer.as_ref(), timer_slot) {
            timer.arm(slot);
        }
        self.pop_scope(scope)?;
        self.finish_frame(FrameKind::Live);
        let gtao_us = if gtao_timed {
            self.live_gtao_row_us()
        } else {
            stale_us
        };
        self.gtao_submit_us = gtao_us;
        // The count tallies the blocking probe tranche and the stale reuse; the
        // live device pass reports its time in the cell alone.
        let gtao_passes = u32::from(stale_us > 0);
        let mut receipt = self.frame_receipt(
            list.items.len(),
            &LookPassReceipt::skipped(),
            ibl,
            (0, 0, 0, gtao_passes, 0, gtao_us),
        );
        if sky_draw {
            receipt.sky_submit_us = self.live_sky_row_us();
        }
        Ok(receipt)
    }

    /// The opt-in probe frame (`set_look_probes`): the earlier window
    /// frame with its blocking tranche. The windowed forward path records
    /// the same look tranche as the offscreen twin: sky, layers, particles
    /// and decals under one shared look bind before the PBR draws, then
    /// the IBL chart plus GTAO plus the fog tranche from this frame's depth
    /// feed, composited by the blit. The probes (bind probes plus chart)
    /// feed no pixel, so the window may space them out; every frame that
    /// ran any of them is tallied as a probe frame.
    fn present_frame_probed(
        &mut self,
        sink: LiveSink<'_>,
        list: &DrawList,
        width: u32,
        height: u32,
    ) -> Result<FrameReceipt, PresentError> {
        let mut order_buf = [0usize; MAX_DRAWS_PER_FRAME];
        let order = Self::frame_draw_order(list, &mut order_buf);
        let frame = effective_camera(list, width, height)?;
        let (cam_bytes, light_bytes) =
            self.stage_frame_with_camera(list, &frame, frame.built_view_proj, order)?;
        self.frame_waits = FrameWaits::default();
        let probe = self
            .frames_presented
            .is_multiple_of(self.probe_every.max(1));
        // QL12: the probe frame times its particle pass; a held frame runs no
        // probe pass and takes no timer slot.
        let timer_slot = if probe {
            self.gpu_timer
                .as_mut()
                .and_then(|timer| timer.begin_frame(&self.device))
        } else {
            None
        };
        let look = if probe {
            self.encode_look_passes(&frame)?
        } else {
            LookPassReceipt::skipped()
        };
        // Fresh targets hold no fog pair yet: refresh on their first frame.
        let fresh_targets = self
            .targets
            .as_ref()
            .is_none_or(|t| t.width != width || t.height != height);
        self.ensure_targets(width, height);
        self.write_frame_uniforms(&cam_bytes, &light_bytes, true);
        // The surface twin samples the same cascaded sun as the
        // offscreen twin; the frame focus depth selects the band.
        self.bind_twin_shadow(Self::twin_focus_depth(list))?;
        self.write_fog_forward(&frame, false)?;
        let fog_wired = self.fog_enabled && super::super::fog_fits_frame(width, height);
        // The fog tranche (depth readback, CPU decode, fog plus contact
        // compute, readback, upload) is the blocking part of a frame; the
        // window may refresh it every `fog_every` frames and blit the
        // last pair in between.
        let fog_due = fresh_targets || self.frames_presented.is_multiple_of(self.fog_every.max(1));
        let fog_now = fog_wired && fog_due;
        if fog_wired {
            let params = self.look_fog_params()?;
            let contact = super::super::contact_for_frame(width, height)?;
            self.ensure_fog_plan(&params, &contact, width, height)?;
        }
        self.ensure_depth_rb(width, height);
        // Per-frame draw binds (base-color resolve after staging).
        self.resolve_draw_binds(list)?;

        // LF3 landing 2: the probe frame runs the same occlusion field as
        // the live frame, so the window and the capture agree.
        let gtao_run = self.gtao_live_plan(true, width, height);
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ra1-frame"),
            });
        // QL12: only a probe frame stages a live particle batch, so only a
        // probe frame records the particle pass and writes its timestamps.
        let particle_timed = probe && self.particle_sub.staged_count() > 0;
        {
            let targets = self.targets.as_ref().expect("ensured above");
            let (_, _, depth_rb) = self.depth_rb.as_ref().expect("ensured above");
            let timer = self.gpu_timer.as_ref().zip(timer_slot);
            self.encode_ao_field(&mut enc, targets, list, &frame, gtao_run, None)?;
            self.encode_forward(
                &mut enc,
                targets,
                list,
                ForwardLook::probed(particle_timed, timer.map(|(timer, _)| timer)),
            );
            if let (Some((timer, slot)), true) = (timer, probe) {
                timer.encode_resolve(
                    &mut enc,
                    slot,
                    PassesTimed {
                        sky: false,
                        gtao: false,
                        particle: particle_timed,
                    },
                );
            }
            if fog_now {
                enc.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: &targets.depth_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::DepthOnly,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: depth_rb,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(padded_depth_row(width)),
                            rows_per_image: Some(height),
                        },
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        self.queue.submit(Some(enc.finish()));
        // QL12: arm the late read of this frame's particle timestamps; the
        // read never waits, so the frame keeps its non-blocking shape.
        if let (Some((timer, slot)), true) = (self.gpu_timer.as_ref().zip(timer_slot), probe) {
            timer.arm(slot);
        }
        let mut raw_depth = std::mem::take(&mut self.depth_decode);
        // The depth copy above was recorded only when the fog tranche is due,
        // so map only then: a held frame runs no copy and must not map, or a
        // frame tallied live pays one map plus one poll wait, the stall the
        // arena smoke fails on (`WaitStats::live_blocking`).
        if fog_now {
            let (_, _, depth_rb) = self.depth_rb.as_ref().expect("ensured above");
            map_readback_f32(
                &self.device,
                &mut self.frame_waits,
                depth_rb,
                width,
                padded_depth_row(width),
                height,
                &mut raw_depth,
            )?;
        }
        let fog_rows: FogRows = if fog_now {
            let rows = self.run_fog_tranche(&raw_depth, &frame, width, height);
            self.depth_decode = raw_depth;
            rows?
        } else if fog_wired {
            self.depth_decode = raw_depth;
            // Between refreshes the blit keeps the last fog pair.
            (0, 0, 0, 0, 0, 0)
        } else if self.fog_pair_dirty {
            self.depth_decode = raw_depth;
            self.clear_fog_targets(width, height)
        } else {
            // Pair already neutral: skip the two full-res re-uploads
            // plus the uniform write this frame.
            self.depth_decode = raw_depth;
            (0, 0, 0, 0, 0, 0)
        };
        // Post-forward chart: same bake, same submitter, own receipt
        // rows, so the look tranche counts exactly the four forward
        // inputs while the chart still submits on every probe frame.
        let ibl = if probe {
            self.submit_ibl_chart()?
        } else {
            (0, 0, 0)
        };
        let mut blit_enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ra1-surface-blit"),
            });
        {
            let targets = self.targets.as_ref().expect("ensured above");
            // Cached at attach: no per-frame pipeline build, and a failed
            // attach never replaced the last-good one.
            let (blit_view, blit_pipeline) = self.sink_target(sink)?;
            self.encode_blit(
                &mut blit_enc,
                &targets.blit_bind,
                blit_view,
                blit_pipeline,
                None,
            );
        }
        self.queue.submit(Some(blit_enc.finish()));
        self.pop_scope(scope)?;
        let kind = if probe || fog_now {
            FrameKind::Probe
        } else {
            FrameKind::Live
        };
        self.finish_frame(kind);
        Ok(self.frame_receipt(list.items.len(), &look, ibl, fog_rows))
    }
}

/// Held by every test that flips the process-wide half-res switch and by
/// the GPU tests that compare frames, so a frame never renders at half size
/// because another test raised the switch for a moment. Test only.
#[cfg(test)]
static HALF_RES_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod live_half_res_tests {
    use super::{live_half_res_enabled, live_present_size, HALF_RES_TEST_LOCK, LIVE_HALF_RES};

    fn set_half(on: bool) {
        LIVE_HALF_RES.store(on, core::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn off_keeps_full_pixels_on_halves_them() {
        let _held = HALF_RES_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        set_half(false);
        assert!(!live_half_res_enabled(), "switch defaults off");
        assert_eq!(live_present_size(1280, 720, false), (1280, 720));
        assert_eq!(
            live_present_size(1280, 720, live_half_res_enabled()),
            (1280, 720)
        );
        set_half(true);
        assert!(live_half_res_enabled(), "switch reads back on");
        assert_eq!(live_present_size(1280, 720, true), (640, 360));
        assert_eq!(
            live_present_size(1280, 720, live_half_res_enabled()),
            (640, 360)
        );
        set_half(false);
        assert!(!live_half_res_enabled(), "switch reads back off");
    }

    #[test]
    fn half_ceils_tiny_and_zero_passes_through() {
        assert_eq!(live_present_size(3, 5, true), (2, 3));
        assert_eq!(live_present_size(1, 1, true), (1, 1));
        assert_eq!(live_present_size(0, 0, true), (0, 0));
        assert_eq!(live_present_size(0, 720, true), (0, 720));
        assert_ne!(live_present_size(1920, 1080, true), (1920, 1080));
        assert_eq!(live_present_size(1920, 1080, true), (960, 540));
    }
}

#[cfg(test)]
mod live_bloom_gate_tests {
    use super::{
        live_bloom_enabled, live_bloom_for_frame, live_bloom_should_run, set_live_bloom_enabled,
    };

    #[test]
    fn live_bloom_matches_post_gate_across_thresholds() {
        // Caller: future live bloom pass (parity with the post oracle).
        // Behavior: the live gate never drifts from `bloom_should_run`.
        assert!(!live_bloom_should_run(0.06, 1.0, 0.5), "dark stays off");
        assert_eq!(
            live_bloom_should_run(0.06, 1.0, 0.5),
            crate::post::bloom_should_run(0.06, 1.0, 0.5)
        );
        assert!(live_bloom_should_run(4.0, 1.0, 0.5), "torch runs");
        assert_eq!(
            live_bloom_should_run(4.0, 1.0, 0.5),
            crate::post::bloom_should_run(4.0, 1.0, 0.5)
        );
        for (luma, threshold, knee) in [
            (f32::NAN, 1.0, 0.5),
            (f32::INFINITY, 1.0, 0.5),
            (4.0, -1.0, 0.5),
            (4.0, 1.0, -0.1),
            (4.0, 1.0, 1.1),
            (4.0, 1.0, f32::NAN),
        ] {
            assert!(!live_bloom_should_run(luma, threshold, knee));
            assert_eq!(
                live_bloom_should_run(luma, threshold, knee),
                crate::post::bloom_should_run(luma, threshold, knee)
            );
        }
        assert_eq!(
            live_bloom_should_run(0.06, 1.0, 0.5),
            live_bloom_should_run(0.06, 1.0, 0.5)
        );
        assert_eq!(
            live_bloom_should_run(4.0, 1.0, 0.5),
            live_bloom_should_run(4.0, 1.0, 0.5)
        );
    }

    #[test]
    fn live_bloom_for_frame_gates_torch_and_dark() {
        // Caller: future live bloom pass (switch plus gate).
        // Behavior: off never runs, on follows the post oracle.
        set_live_bloom_enabled(false);
        assert!(!live_bloom_enabled(), "switch reads back off");
        assert!(
            !live_bloom_for_frame(false, 4.0, 1.0, 0.5),
            "off plus torch stays off"
        );
        assert!(!live_bloom_for_frame(live_bloom_enabled(), 4.0, 1.0, 0.5));
        set_live_bloom_enabled(true);
        assert!(live_bloom_enabled(), "switch reads back on");
        assert!(
            !live_bloom_for_frame(true, 0.05, 1.0, 0.5),
            "on plus dark stays off"
        );
        assert!(
            live_bloom_for_frame(true, 4.0, 1.0, 0.5),
            "on plus torch runs"
        );
        for (luma, threshold, knee) in
            [(f32::NAN, 1.0, 0.5), (4.0, -1.0, 0.5), (4.0, 1.0, f32::NAN)]
        {
            assert!(!live_bloom_for_frame(true, luma, threshold, knee));
        }
        assert_eq!(
            live_bloom_for_frame(true, 4.0, 1.0, 0.5),
            live_bloom_for_frame(true, 4.0, 1.0, 0.5)
        );
        set_live_bloom_enabled(false);
        assert!(!live_bloom_enabled(), "switch restores off");
    }

    #[test]
    fn live_bloom_gates_halo_dark_stays_identical() {
        // Caller: `present_frame_live` live bloom pass (switch plus gate).
        // Behavior: off never runs, on follows the oracle; dark stays off,
        // torch runs; the live frame consults the gate before the blit.
        set_live_bloom_enabled(false);
        assert!(!live_bloom_enabled(), "switch reads back off");
        assert!(
            !live_bloom_for_frame(false, 0.05, 1.0, 0.5),
            "off plus dark stays off"
        );
        assert!(
            !live_bloom_for_frame(false, 4.0, 1.0, 0.5),
            "off plus torch stays off"
        );
        assert!(!live_bloom_for_frame(live_bloom_enabled(), 4.0, 1.0, 0.5));
        set_live_bloom_enabled(true);
        assert!(live_bloom_enabled(), "switch reads back on");
        assert!(
            !live_bloom_for_frame(true, 0.05, 1.0, 0.5),
            "on plus dark stays off"
        );
        assert!(
            live_bloom_for_frame(true, 4.0, 1.0, 0.5),
            "on plus torch runs"
        );
        let src = include_str!("live.rs");
        let live_fn = src
            .find("fn present_frame_live")
            .expect("live frame exists");
        let live_end = src[live_fn..]
            .find("fn present_frame_probed")
            .map(|i| live_fn + i)
            .unwrap_or(src.len());
        let live_body = &src[live_fn..live_end];
        assert!(
            live_body.contains("live_bloom_for_frame"),
            "live path consults the gate"
        );
        assert!(live_body.contains("encode_blit"), "live path blits");
        let gate_at = live_body
            .find("live_bloom_for_frame")
            .expect("gate sits in live frame");
        let blit_at = live_body
            .find("encode_blit")
            .expect("blit sits in live frame");
        assert!(gate_at < blit_at, "gate runs before the blit");
        set_live_bloom_enabled(false);
        assert!(!live_bloom_enabled(), "switch restores off");
    }

    #[test]
    fn live_bloom_switch_ships_on_in_the_window_frame() {
        // Caller: `present_frame_live` (the shipped window frame; no game
        // caller raises the switch today).
        // Behavior: the live switch ships at `LIVE_BLOOM_DEFAULT_ON`, which
        // mirrors the presenter's `BloomState::on()` default, so a bright
        // arena frame runs the halo in the window; a dark frame and the off
        // leg stay unchanged.
        const {
            assert!(
                super::LIVE_BLOOM_DEFAULT_ON,
                "the window frame ships blooming"
            );
        }
        assert!(
            include_str!("presenter.rs").contains("bloom: BloomState::on()"),
            "the presenter default is the window frame's bloom state"
        );
        assert!(
            live_bloom_for_frame(
                super::LIVE_BLOOM_DEFAULT_ON,
                2.4,
                super::LIVE_BLOOM_THRESHOLD,
                super::LIVE_BLOOM_KNEE
            ),
            "a bright frame runs the halo with the shipped thresholds"
        );
        assert!(
            !live_bloom_for_frame(
                super::LIVE_BLOOM_DEFAULT_ON,
                0.05,
                super::LIVE_BLOOM_THRESHOLD,
                super::LIVE_BLOOM_KNEE
            ),
            "a dark frame stays off even with the switch shipped on"
        );
        assert!(
            !live_bloom_for_frame(false, 2.4, 1.0, 0.5),
            "the off leg stays off"
        );
    }
}

#[cfg(test)]
mod live_bloom_glint_tests {
    use super::*;
    use crate::test_gpu::{require_or_skip, NoAdapter};

    const SIDE: u32 = 128;
    /// Bright enough to pass the knee, dim enough that no edge texel's
    /// four-neighbour average does (an average-then-threshold halo misses it).
    const GLINT_EXPOSURE: f32 = 6.0;

    fn glint_key() -> MeshKey {
        MeshKey::new(0x6C1A, [0x6C; 32])
    }

    fn init_or_skip(test: &str) -> Option<Presenter> {
        let probe = match Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(PresentError::NoAdapter) => {
                Err(NoAdapter("no wgpu adapter on this machine".into()))
            }
            Err(e) => panic!("presenter init failed (must fail loudly): {e}"),
        };
        require_or_skip(test, probe)
    }

    /// A unit cube 40 m out: a few pixels wide on a 128 px frame, lit to
    /// `exposure` times the default light.
    fn glint_list(exposure: f32) -> DrawList {
        let scene = crate::v1::default_scene(SIDE, SIDE).expect("default scene is valid");
        DrawList {
            camera: crate::v1::Camera::new(
                [0.0, 0.0, 40.0],
                [0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                40.0f32.to_radians(),
            ),
            light: scene.light,
            items: vec![DrawItem {
                mesh: glint_key(),
                material: crate::v1::PbrMaterial {
                    base_color: [1.0, 1.0, 1.0],
                    metallic: 0.0,
                    roughness: 0.8,
                    exposure,
                },
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            }],
        }
    }

    /// Copy the LDR twin to the CPU as tight RGBA8 rows. Test only: waits
    /// on the GPU.
    fn read_twin(presenter: &Presenter) -> Vec<u8> {
        let targets = presenter.targets.as_ref().expect("twin targets exist");
        let (w, h) = (targets.width, targets.height);
        let padded = (w * 4).div_ceil(256) * 256;
        let buffer = presenter.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test-glint-readback"),
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = presenter
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &targets.out_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
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
        presenter.queue.submit(Some(enc.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |r| {
            r.expect("test readback maps");
        });
        presenter
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("test readback polls");
        let row = (w * 4) as usize;
        let mut rgba = vec![0u8; row * h as usize];
        {
            let mapped = buffer.slice(..).get_mapped_range().expect("mapped");
            for (y, dst) in rgba.chunks_exact_mut(row).enumerate() {
                let off = y * padded as usize;
                dst.copy_from_slice(&mapped[off..off + row]);
            }
        }
        buffer.unmap();
        rgba
    }

    fn render(presenter: &mut Presenter, list: &DrawList) -> Vec<u8> {
        let receipt = presenter
            .render_live_offscreen(list)
            .expect("glint frame renders");
        assert!(receipt.presented && receipt.is_clean());
        assert_eq!(receipt.validation_errors, 0, "glint frame stays clean");
        read_twin(presenter)
    }

    fn differs(a: &[u8], b: &[u8], texel: usize, min: u8) -> bool {
        (0..3).any(|c| a[texel * 4 + c].abs_diff(b[texel * 4 + c]) >= min)
    }

    /// Texels within `reach` (Chebyshev) of a set texel of `mask`.
    fn near(mask: &[bool], reach: usize) -> Vec<bool> {
        let side = SIDE as usize;
        let mut out = vec![false; mask.len()];
        for y in 0..side {
            for x in 0..side {
                if !mask[y * side + x] {
                    continue;
                }
                for ny in y.saturating_sub(reach)..=(y + reach).min(side - 1) {
                    for nx in x.saturating_sub(reach)..=(x + reach).min(side - 1) {
                        out[ny * side + nx] = true;
                    }
                }
            }
        }
        out
    }

    #[test]
    fn glint_haloes_beyond_its_own_pixels_and_dark_corner_stays() {
        // Caller: `present_frame_live` flag write into the blit bloom.
        // Behavior: a glint a few pixels wide, whose 4-neighbour average
        // stays under the knee, still throws a halo past its own footprint
        // (extract before blur); the far corner and the replay do not move.
        let Some(mut presenter) =
            init_or_skip("glint_haloes_beyond_its_own_pixels_and_dark_corner_stays")
        else {
            return;
        };
        let _held = super::HALF_RES_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        presenter
            .upload_mesh(glint_key(), &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(SIDE, SIDE).expect("valid size applies");
        // The dim frame (same cube, exposure near zero) marks the glint's
        // own texels: whatever the lit frame changes against it.
        presenter.set_bloom_enabled(false);
        let dim = render(&mut presenter, &glint_list(0.02));
        let lit = glint_list(GLINT_EXPOSURE);
        let off = render(&mut presenter, &lit);
        presenter.set_bloom_enabled(true);
        let on = render(&mut presenter, &lit);
        let replay = render(&mut presenter, &lit);
        assert_eq!(on, replay, "the same glint replays byte-identical");
        let texels = (SIDE * SIDE) as usize;
        let body: Vec<bool> = (0..texels).map(|t| differs(&off, &dim, t, 24)).collect();
        let body_count = body.iter().filter(|b| **b).count();
        assert!(
            (4..=64).contains(&body_count),
            "the glint is a few pixels wide, got {body_count}"
        );
        // Texels two or more away from any glint texel: only a halo that
        // spills past the glint's own neighbours reaches them.
        let reach = near(&body, 1);
        let spill = (0..texels)
            .filter(|t| !reach[*t] && differs(&off, &on, *t, 3))
            .count();
        assert!(
            spill >= 12,
            "the glint's halo reaches past its neighbours: {spill} texels"
        );
        let side = SIDE as usize;
        for y in 0..8usize {
            for x in 0..8usize {
                let t = y * side + x;
                assert!(
                    !differs(&off, &on, t, 1),
                    "a texel far from the glint is unchanged at ({x}, {y})"
                );
            }
        }
        presenter.set_bloom_enabled(false);
    }

    #[test]
    fn the_meter_adapts_the_exposure_and_the_blit_carries_it() {
        // GS44 end to end: the GPU histogram of the HDR target lands one
        // frame late, the meter adapts the stops, and the next frame's blit
        // carries the multiplier. Fail-closed: before the first histogram
        // the multiplier is exactly 1.0, so no frame moves by accident.
        let Some(mut presenter) = init_or_skip("meter_adapts_exposure") else {
            return;
        };
        let _held = super::HALF_RES_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        presenter
            .upload_mesh(glint_key(), &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(SIDE, SIDE).expect("valid size applies");
        presenter.set_bloom_enabled(false);
        presenter.set_auto_exposure_enabled(true);
        assert_eq!(
            presenter.auto_exposure_mult(),
            1.0,
            "no histogram yet: the meter never guesses"
        );
        let list = glint_list(GLINT_EXPOSURE);
        let mut stops = Vec::new();
        for _ in 0..6 {
            let _ = render(&mut presenter, &list);
            stops.push(presenter.auto_exposure_ev());
        }
        eprintln!("auto exposure stops: {stops:?}");
        assert!(
            presenter.auto_exposure_histogram().is_some(),
            "the GPU histogram of the HDR target landed"
        );
        assert!(
            stops.iter().all(|ev| ev.is_finite()),
            "the stops stay finite: {stops:?}"
        );
        let mult = presenter.auto_exposure_mult();
        assert!(
            (mult - 1.0).abs() > 1e-4,
            "the meter moved the exposure off neutral: {mult}"
        );
        // Off is exactly neutral again: the switch is the only effect.
        presenter.set_auto_exposure_enabled(false);
        assert_eq!(presenter.auto_exposure_mult(), 1.0, "off is 0 EV");
    }
}

#[cfg(test)]
mod live_wall_split_tests {
    use super::super::frame::{last_live_wall_split, LiveWallSplit};
    use super::*;
    use crate::test_gpu::{require_or_skip, NoAdapter};

    #[test]
    fn live_wall_split_sums_and_prints_the_three_spans() {
        // Behavior: the total is the saturating sum and the summary names
        // each span in report order with three decimals.
        let split = LiveWallSplit {
            stage_us: 1_250,
            submit_us: 2_005,
            present_call_us: 40,
        };
        assert_eq!(split.total_us(), 3_295);
        assert_eq!(
            split.summary(),
            "stage=1.250ms submit=2.005ms present=0.040ms"
        );
        assert_eq!(LiveWallSplit::default().total_us(), 0, "zero by default");
        let huge = LiveWallSplit {
            stage_us: u64::MAX,
            submit_us: 1,
            present_call_us: 1,
        };
        assert_eq!(huge.total_us(), u64::MAX, "the sum saturates");
    }

    #[test]
    fn live_wall_split_spans_sit_where_the_packet_names_them() {
        // Source pin: the stage clock opens at entry and closes before the
        // encoder opens, the submit clock covers the encoder through
        // `queue.submit`, and the surface path times `queue.present` alone.
        let src = include_str!("live.rs");
        let live = src
            .find("fn present_frame_live")
            .expect("live frame exists");
        let probed = src
            .find("fn present_frame_probed")
            .expect("probe frame exists");
        let body = &src[live..probed];
        let at = |needle: &str| at_in(body, needle);
        assert!(at("let stage_started") < at("self.stage_frame_with_camera("));
        assert!(at("self.resolve_draw_binds(list)") < at("let stage_us = micros_since"));
        assert!(at("let stage_us = micros_since") < at("let submit_started"));
        assert!(at("let submit_started") < at("create_command_encoder"));
        assert!(at("self.queue.submit(") < at("submit_us: micros_since(submit_started)"));
        let from = src
            .find("pub(super) fn present_frame(")
            .expect("present_frame exists");
        let present = &src[from..live];
        let opens = at_in(present, "let present_started");
        let calls = at_in(present, "self.queue.present(frame)");
        let closes = at_in(
            present,
            "record_present_call_us(micros_since(present_started))",
        );
        assert!(
            opens < calls && calls < closes,
            "only the present call sits inside the third span"
        );
    }

    fn at_in(text: &str, needle: &str) -> usize {
        text.find(needle)
            .unwrap_or_else(|| panic!("missing {needle}"))
    }

    #[test]
    fn live_stage_split_counts_each_half() {
        // Caller: `present_frame_live`. Behavior: a window-path frame
        // reports a stage span and a submit span, both above zero, and the
        // three together fit inside the wall of the call. The offscreen twin
        // has no swapchain image, so its present span is zero.
        let probe = match Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(PresentError::NoAdapter) => {
                Err(NoAdapter("no wgpu adapter on this machine".into()))
            }
            Err(e) => panic!("presenter init failed (must fail loudly): {e}"),
        };
        let Some(mut presenter) = require_or_skip("live_stage_split_counts_each_half", probe)
        else {
            return;
        };
        let _held = HALF_RES_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = MeshKey::new(0x57A6, [0x57; 32]);
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(128, 128).expect("valid size applies");
        let scene = crate::v1::default_scene(128, 128).expect("default scene is valid");
        let list = DrawList {
            camera: scene.camera,
            light: scene.light,
            items: vec![DrawItem {
                mesh: key,
                material: scene.material,
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            }],
        };
        for frame in 0..3 {
            let started = engine_clock::now();
            let receipt = presenter
                .render_live_offscreen(&list)
                .expect("live frame renders");
            let wall_us = micros_since(started);
            assert!(receipt.presented && receipt.is_clean());
            let split = last_live_wall_split();
            eprintln!("live wall split frame {frame}: {}", split.summary());
            assert!(split.stage_us > 0, "frame {frame}: staging is timed");
            assert!(
                split.submit_us > 0,
                "frame {frame}: encode and submit are timed"
            );
            assert_eq!(split.present_call_us, 0, "the twin makes no present call");
            assert!(
                split.total_us() <= wall_us,
                "frame {frame}: spans {} fit the call wall {wall_us}",
                split.total_us()
            );
        }
    }
}

#[cfg(test)]
mod live_ibl_receipt_tests {
    #[test]
    fn live_frame_reports_ibl_above_zero() {
        // Caller: `present_frame_live` receipt path.
        // Behavior: live receipt reports bound IBL encode above zero.
        let src = include_str!("live.rs");
        assert!(
            src.contains("live_ibl_receipt"),
            "live receipt times bound encode"
        );
        assert!(
            !src.contains("&LookPassReceipt::skipped(),\n            (0, 0, 0)"),
            "live ibl no longer hardcoded zero"
        );
        assert!(
            crate::ibl::live_ibl_receipt(0.42, 64, 64).2 > 0,
            "live ibl above zero"
        );
        assert_eq!(
            crate::ibl::live_ibl_receipt(0.42, 0, 64),
            (0, 0, 0),
            "zero size stays skipped"
        );
    }

    #[test]
    fn live_ibl_span_covers_the_forward_encode_only() {
        // Caller: `present_frame_live` receipt path.
        // Behavior: the clock that becomes `ibl.2` opens at the forward
        // IBL-bound encode and closes on it, so the AO upload, the blit and
        // the overlay below never bill to the `ibl` column.
        let src = include_str!("live.rs");
        let open = "let forward_ibl_started = engine_clock::now();";
        let close = "engine_clock::since(forward_ibl_started)";
        let start = src.find(open).expect("live path opens the ibl clock");
        let end = src.find(close).expect("live path closes the ibl clock");
        assert!(start < end, "the ibl clock closes after it opens");
        let span = &src[start..end];
        assert!(
            span.contains("self.encode_forward("),
            "the span holds the forward IBL-bound encode"
        );
        for leak in [
            "create_command_encoder",
            "live_ao_upload_for_frame(",
            "stale_ao_reuse_upload(",
            "encode_blit(",
            "encode_overlay(",
            "encode_resolve(",
        ] {
            assert!(!span.contains(leak), "the ibl span leaks {leak}");
        }
    }

    #[test]
    fn live_ibl_samples_bake_only_when_bound_and_nonzero_live_gate() {
        // Caller: `present_frame_live` gated receipt path.
        // Behavior: unbound or unlit stays skipped, bound plus lit above zero.
        use super::live_ibl_for_frame;
        assert_eq!(live_ibl_for_frame(false, true, 0.42, 64, 64), (0, 0, 0));
        assert_eq!(live_ibl_for_frame(true, false, 0.42, 64, 64), (0, 0, 0));
        let bound = live_ibl_for_frame(true, true, 0.42, 64, 64);
        assert!(bound.2 > 0, "bound plus lit stays above zero");
        assert_eq!(bound, crate::ibl::live_ibl_receipt(0.42, 64, 64));
        assert_eq!(live_ibl_for_frame(true, true, 0.42, 0, 64), (0, 0, 0));
        assert_eq!(
            live_ibl_for_frame(true, true, 0.42, 64, 64),
            live_ibl_for_frame(true, true, 0.42, 64, 64)
        );
        let src = include_str!("live.rs");
        assert!(
            src.contains("live_ibl_for_frame"),
            "live path gates receipt"
        );
        assert!(src.contains("bake_has_light"), "live reads bake verdict");
    }

    #[test]
    fn live_ibl_gate_counts_only_when_bound_lit_timed_and_moved() {
        // Caller: `present_frame_live` receipt via `live_ibl_should_count`.
        // Behavior: unbound/unlit/untimed/tiny-gap stays skip, all present counts.
        use super::live_ibl_should_count;
        assert!(
            !live_ibl_should_count(false, true, 0.42, 0.5),
            "unbound refuses"
        );
        assert!(
            !live_ibl_should_count(true, false, 0.42, 0.5),
            "unlit refuses"
        );
        assert!(!live_ibl_should_count(true, true, 0.0, 0.5), "0 ms refuses");
        assert!(
            !live_ibl_should_count(true, true, 0.42, 0.002),
            "epsilon edge refuses"
        );
        assert!(
            !live_ibl_should_count(true, true, 0.42, 0.001),
            "tiny gap refuses"
        );
        assert!(
            live_ibl_should_count(true, true, 0.42, 0.5),
            "all present counts"
        );
        assert_eq!(
            live_ibl_should_count(true, true, 0.42, 0.5),
            live_ibl_should_count(true, true, 0.42, 0.5),
            "same inputs agree twice"
        );
        let src = include_str!("live.rs");
        assert!(
            src.contains("live_ibl_should_count"),
            "live receipt gates ibl time"
        );
        assert!(
            src.contains("live_ibl_should_sample"),
            "gate delegates to sample rule"
        );
    }
}

#[cfg(test)]
mod live_sky_gate_tests {
    use super::live_sky_should_count;

    #[test]
    fn live_sky_gate_needs_both_time_and_gap() {
        // Caller: `present_frame_live` receipt path.
        // Behavior: zero time or zero gap stays skip, both present counts.
        assert!(!live_sky_should_count(0.0, 0.5), "zero time refuses");
        assert!(!live_sky_should_count(0.42, 0.0), "zero gap refuses");
        assert!(!live_sky_should_count(0.42, 0.01), "epsilon edge refuses");
        assert!(live_sky_should_count(0.42, 0.5), "bound plus moved counts");
        let src = include_str!("live.rs");
        assert!(
            src.contains("live_sky_should_count"),
            "live receipt gates sky time"
        );
        assert!(
            src.contains("sky_live_on_off_gap"),
            "live receipt reads the sky gap oracle"
        );
    }
}

#[cfg(test)]
mod frame_camera_once_tests {
    #[test]
    fn live_frame_builds_camera_once_and_shares_bytes() {
        // Caller: `present_frame_live` / `present_frame_probed` plus `render_offscreen` in `submit`.
        // Behavior: one `effective_camera` build per frame; shared camera matches rebuilt bytes.
        let src = include_str!("live.rs");
        assert!(src.contains("fn write_fog_forward"), "fog feed exists");
        assert!(
            src.contains("camera: &v1::Camera") || src.contains("camera: &crate::v1::Camera"),
            "fog feed takes the shared frame camera"
        );
        let at = src.find("fn write_fog_forward").expect("fog feed exists");
        let end = (at + 900).min(src.len());
        assert!(
            !src[at..end].contains("effective_camera"),
            "fog feed reuses the frame camera instead of rebuilding it"
        );
        assert!(
            src.contains("stage_frame_with_camera"),
            "staging reuses the shared camera"
        );
        // Built in two halves so this pin never matches its own text.
        let rebuild_call = ["stage_frame(", "list,"].concat();
        assert!(
            !src.contains(&rebuild_call),
            "live paths no longer rebuild via stage_frame"
        );
        let scene = crate::v1::default_scene(64, 64).expect("default scene is valid");
        let list = crate::present::DrawList {
            camera: scene.camera.clone(),
            light: scene.light.clone(),
            items: Vec::new(),
        };
        let once =
            super::super::submit::effective_camera(&list, 64, 64).expect("frame camera builds");
        let rebuilt =
            super::super::submit::effective_camera(&list, 64, 64).expect("rebuilt camera builds");
        assert_eq!(once.eye, rebuilt.eye, "shared eye matches rebuilt");
        assert_eq!(once.aspect, rebuilt.aspect, "shared aspect matches rebuilt");
        let vp_once = once.view_proj().expect("shared view-proj builds");
        let vp_rebuilt = rebuilt.view_proj().expect("rebuilt view-proj builds");
        assert_eq!(
            vp_once.to_cols_array(),
            vp_rebuilt.to_cols_array(),
            "shared view-proj matches rebuilt"
        );
        let inv_once =
            super::super::submit::inverse_view_proj_cols(&once).expect("shared inverse builds");
        let inv_rebuilt =
            super::super::submit::inverse_view_proj_cols(&rebuilt).expect("rebuilt inverse builds");
        assert_eq!(inv_once, inv_rebuilt, "shared inverse matches rebuilt");
        let (cam_once, _, _) =
            crate::v1::build_uniforms(&once, once.aspect, &scene.light, &scene.material)
                .expect("shared uniforms build");
        let (cam_rebuilt, _, _) =
            crate::v1::build_uniforms(&rebuilt, rebuilt.aspect, &scene.light, &scene.material)
                .expect("rebuilt uniforms build");
        assert_eq!(
            cam_once.to_array(),
            cam_rebuilt.to_array(),
            "shared staged bytes match rebuilt"
        );
    }
}

#[cfg(test)]
mod one_view_proj_frame_build_tests {
    /// Production head of a file: everything before its first test module.
    fn head(src: &str) -> &str {
        &src[..src.find("#[cfg(test)]").unwrap_or(src.len())]
    }

    #[test]
    fn one_camera_view_proj_build_per_frame() {
        // Caller: `present_frame_live` / `present_frame_probed` here plus
        // `render_offscreen` in `submit`.
        // Behavior: the frame path builds the camera view-projection once per
        // frame and every later reader takes the built matrix instead of
        // building the matrix again.
        let build = concat!("camera.view_proj", "()");
        let access = concat!(".view_proj", "()");
        let live = head(include_str!("live.rs"));
        assert_eq!(
            live.matches(access).count(),
            0,
            "live frame path builds no view-proj"
        );
        assert!(
            live.contains("built_view_proj"),
            "live staging reads the built matrix"
        );
        assert!(
            live.contains("build_uniforms_with_view_proj"),
            "live staging packs the built matrix"
        );
        let fog = head(include_str!("fog_tranche.rs"));
        assert_eq!(
            fog.matches(access).count(),
            0,
            "fog path builds no view-proj"
        );
        let submit = include_str!("submit.rs");
        let eff = submit.find("fn effective_camera").expect("build owner");
        let inv = submit
            .find("fn inverse_view_proj_cols")
            .expect("inverse owner");
        assert_eq!(
            submit[eff..inv].matches(build).count(),
            1,
            "the frame camera builds view_proj once"
        );
        assert_eq!(
            submit.matches(build).count(),
            1,
            "no other build sits on the frame path"
        );
        assert_eq!(
            submit[inv..].matches(build).count(),
            0,
            "the fog inverse builds no matrix"
        );
        assert!(
            submit[inv..].contains("built_view_proj"),
            "the fog inverse inverts the built matrix"
        );
        assert!(
            submit.contains("build_uniforms_with_view_proj"),
            "capture staging packs the built matrix"
        );
        let v1 = include_str!("../../v1.rs");
        let pack = v1
            .find("pub fn build_uniforms_with_view_proj(")
            .expect("the packer takes the built matrix");
        let mat = v1
            .find("pub(crate) fn build_material(")
            .expect("the material packer follows it");
        assert_eq!(
            v1[pack..mat].matches(access).count(),
            0,
            "the built-matrix packer never rebuilds view_proj"
        );
    }
}

#[cfg(test)]
mod live_bind_skip_tests {
    use super::{live_bind_should_skip, live_bind_skip_enabled, set_live_bind_skip_enabled};

    fn bind_list(mesh_id: u64) -> crate::present::DrawList {
        let scene = crate::v1::default_scene(160, 120).expect("valid scene");
        crate::present::DrawList {
            camera: scene.camera,
            light: scene.light,
            items: vec![crate::present::DrawItem {
                mesh: crate::present::MeshKey::new(mesh_id, [0x11; 32]),
                material: scene.material,
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            }],
        }
    }

    #[test]
    fn unchanged_reuses_live_present_path_changed_rebuilds() {
        // Caller: `present_frame_live` live present path.
        // Behavior: unchanged revision reuses binds, changed rebuilds.
        set_live_bind_skip_enabled(false);
        assert!(!live_bind_skip_enabled(), "switch defaults off");
        let list = bind_list(7);
        let rev = super::live_forward_revision(&list);
        assert!(
            !live_bind_should_skip(Some(rev), rev, false),
            "off rebuilds"
        );
        assert!(
            !live_bind_should_skip(None, rev, true),
            "first frame rebuilds"
        );
        assert!(
            live_bind_should_skip(Some(rev), rev, true),
            "unchanged reuses"
        );
        let changed = super::live_forward_revision(&bind_list(8));
        assert_ne!(rev, changed, "changed list moves revision");
        assert!(
            !live_bind_should_skip(Some(rev), changed, true),
            "changed rebuilds"
        );
        set_live_bind_skip_enabled(true);
        assert!(live_bind_skip_enabled(), "switch reads back on");
        assert!(
            live_bind_should_skip(Some(rev), rev, true),
            "on still reuses"
        );
        set_live_bind_skip_enabled(false);
        assert!(!live_bind_skip_enabled(), "switch reads back off");
        let src = include_str!("live.rs");
        assert!(
            src.contains("live_bind_should_skip"),
            "live path checks the skip"
        );
        assert!(
            src.contains("live_bind_last_store"),
            "live path remembers the revision"
        );
        assert!(
            src.contains("present_frame_live"),
            "live present path owns the gate"
        );
    }
}

#[cfg(test)]
mod live_present_skip_tests {
    use super::live_present_should_reuse;

    fn present_list(mesh_id: u64) -> crate::present::DrawList {
        let scene = crate::v1::default_scene(160, 120).expect("valid scene");
        crate::present::DrawList {
            camera: scene.camera,
            light: scene.light,
            items: vec![crate::present::DrawItem {
                mesh: crate::present::MeshKey::new(mesh_id, [0x11; 32]),
                material: scene.material,
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            }],
        }
    }

    #[test]
    fn unchanged_reuses_present_path_changed_or_resized_rebuilds() {
        // Caller: `present_frame_live` live present path.
        // Behavior: unchanged plus matching len reuses, changed or resized rebuilds.
        let list = present_list(7);
        let rev = super::live_forward_revision(&list);
        assert!(
            !live_present_should_reuse(Some(rev), rev, false, 1, 1),
            "off rebuilds"
        );
        assert!(
            !live_present_should_reuse(None, rev, true, 0, 1),
            "first frame rebuilds"
        );
        assert!(
            live_present_should_reuse(Some(rev), rev, true, 1, 1),
            "unchanged reuses"
        );
        assert!(
            !live_present_should_reuse(Some(rev), rev, true, 1, 2),
            "resized rebuilds"
        );
        assert!(
            !live_present_should_reuse(Some(rev), rev, true, 2, 1),
            "shrunk rebuilds"
        );
        let changed = super::live_forward_revision(&present_list(8));
        assert_ne!(rev, changed, "changed list moves revision");
        assert!(
            !live_present_should_reuse(Some(rev), changed, true, 1, 1),
            "changed rebuilds"
        );
        assert!(
            live_present_should_reuse(Some(rev), rev, true, 2, 2),
            "same len reuses"
        );
        assert_eq!(
            live_present_should_reuse(Some(rev), rev, true, 1, 1),
            live_present_should_reuse(Some(rev), rev, true, 1, 1),
            "same inputs agree twice"
        );
        let src = include_str!("live.rs");
        assert!(
            src.contains("live_present_should_reuse"),
            "live path checks the present reuse"
        );
        assert!(
            src.contains("present_frame_live"),
            "live present path owns the gate"
        );
    }
}

#[cfg(test)]
mod live_stale_ao_tests {
    use super::live_ao_should_bind;

    #[test]
    fn live_stale_ao_reuse_darkens_valley() {
        // Caller: `present_frame_live` stale path after `encode_forward`.
        // Behavior: cached field validates, valley darkens, open stays, short and zero skip, twice equal.
        let mut depth = vec![0.0f32; 64];
        for y in 0..8 {
            for x in 0..8 {
                depth[y * 8 + x] = if x < 4 { 2.0 } else { 6.0 };
            }
        }
        let normals = vec![[0.0, 0.0, 1.0]; 64];
        let config = crate::gtao::GtaoConfig {
            width_px: 8,
            height_px: 8,
            radius_px: 4.0,
            falloff_m: 2.0,
            intensity: 1.0,
        };
        let (first, _) =
            crate::gtao::gtao_live_full_for_forward(&depth, &normals, &config).expect("step binds");
        let (second, _) = crate::gtao::gtao_live_full_for_forward(&depth, &normals, &config)
            .expect("step binds again");
        assert_eq!(first, second, "same feed gives same field twice");
        assert!(
            live_ao_should_bind(8, 8, first.len(), first.len()),
            "full field binds"
        );
        assert!(
            crate::gtao::stale_ao_should_reuse(&first, 8, 8),
            "step bake reuses"
        );
        let (valley, open) =
            crate::gtao::ao_on_off_stats(&first, 8, 8, (4, 1), (0, 1)).expect("probes read");
        assert!(
            valley < crate::gtao::GTAO_OPEN,
            "valley must darken, got {valley}"
        );
        assert!(
            (open - crate::gtao::GTAO_OPEN).abs() < 0.02,
            "open sand stays within 2%, got {open}"
        );
        assert!(
            !crate::gtao::stale_ao_should_reuse(&first[..63], 8, 8),
            "short slice stays skip"
        );
        assert!(
            !live_ao_should_bind(0, 8, first.len(), first.len()),
            "zero size stays skip"
        );
        let src = include_str!("live.rs");
        assert!(
            src.contains("stale_ao_should_reuse"),
            "live frame reuses the cached field"
        );
        assert!(
            src.contains("gtao_sub.full_ao"),
            "live frame reads the last probe field"
        );
        assert!(
            src.contains("live_ao_upload_for_frame"),
            "live frame bakes fresh fields through the submit helper"
        );
        assert!(
            src.contains("gtao_submit_us"),
            "live frame records upload time"
        );
    }
}

#[cfg(test)]
mod live_fresh_ao_tests {
    #[test]
    fn live_fresh_ao_binds_after_forward() {
        // Caller: `present_frame_live` fresh path after `encode_forward`.
        // Behavior: step depth through the live oracle darkens the valley
        // while open sand stays open; the live path wires the fresh call.
        let mut depth = vec![0.0f32; 64];
        for y in 0..8 {
            for x in 0..8 {
                depth[y * 8 + x] = if x < 4 { 2.0 } else { 6.0 };
            }
        }
        let normals = vec![[0.0, 0.0, 1.0]; 64];
        let config = crate::gtao::GtaoConfig {
            width_px: 8,
            height_px: 8,
            radius_px: 4.0,
            falloff_m: 2.0,
            intensity: 1.0,
        };
        let (full, _) =
            crate::gtao::gtao_live_full_for_forward(&depth, &normals, &config).expect("step binds");
        let (valley, open) =
            crate::gtao::ao_on_off_stats(&full, 8, 8, (4, 1), (0, 1)).expect("probes read");
        assert!(
            valley < crate::gtao::GTAO_OPEN,
            "valley must darken, got {valley}"
        );
        assert!(
            (open - crate::gtao::GTAO_OPEN).abs() < 0.02,
            "open sand stays within 2%, got {open}"
        );
        let src = include_str!("live.rs");
        let live_fn = src
            .find("fn present_frame_live")
            .expect("live frame exists");
        let live_end = src[live_fn..]
            .find("fn present_frame_probed")
            .map(|i| live_fn + i)
            .unwrap_or(src.len());
        let live_body = &src[live_fn..live_end];
        assert!(
            live_body.contains("live_ao_upload_for_frame"),
            "live path bakes the fresh field through the submit helper after forward"
        );
        assert!(
            live_body.contains("stale_ao_reuse_upload"),
            "live path keeps the stale reuse as fallback"
        );
        let fwd_at = live_body
            .find("encode_forward")
            .expect("forward sits in live frame");
        let helper_at = live_body
            .find("live_ao_upload_for_frame")
            .expect("helper sits in live frame");
        let blit_at = live_body
            .find("encode_blit")
            .expect("blit sits in live frame");
        assert!(
            fwd_at < helper_at && helper_at < blit_at,
            "fresh bake runs after forward and before the blit"
        );
    }
}

#[cfg(test)]
mod frame_gate_tests {
    use super::frame_meets_30fps_gate;

    #[test]
    fn frame_gate_tracks_30fps_bar() {
        // Caller: future quiet-perf gate (FC3 p50<=33.3ms, p99<=50ms).
        // Behavior: both inside bar passes, either over fails, zero passes, twice equal.
        assert!(frame_meets_30fps_gate(0.0, 0.0), "zero stays true");
        assert!(frame_meets_30fps_gate(33.3, 50.0), "edge passes");
        assert!(frame_meets_30fps_gate(16.0, 20.0), "quiet run passes");
        assert!(!frame_meets_30fps_gate(33.4, 20.0), "p50 over fails");
        assert!(!frame_meets_30fps_gate(16.0, 50.1), "p99 over fails");
        assert_eq!(
            frame_meets_30fps_gate(16.0, 20.0),
            frame_meets_30fps_gate(16.0, 20.0),
            "same inputs agree twice"
        );
    }
}

#[cfg(test)]
mod live_overlay_tests {
    #[test]
    fn live_overlay_draws_staged_bars_after_blit() {
        // Caller: `present_frame_live` after-blit path.
        // Behavior: staged HP bar gives one draw, empty gives none; live draws after blit.
        let mut ring = crate::overlay::OverlayRing::new();
        assert!(ring.draw_plan(16.0 / 9.0).is_none(), "empty stays no-op");
        assert!(
            ring.hp_bars_plan(16.0 / 9.0).is_none(),
            "empty hp stays none"
        );
        assert!(
            ring.push_hp_bar(0.04, 0.05, 0.34, 0.09, 0.5),
            "stage one bar"
        );
        let plan = ring.hp_bars_plan(16.0 / 9.0).expect("staged bar draws");
        assert_eq!(plan.quads, 2, "one bar is back plus fill");
        assert_eq!(plan.verts, 8, "two quads give eight verts");
        let mut verts = vec![crate::overlay::QuadVertex::default(); plan.verts];
        assert_eq!(
            ring.write_vertices(&mut verts),
            plan.verts,
            "verts match plan"
        );
        assert_eq!(
            ring.hp_bars_plan(16.0 / 9.0),
            ring.draw_plan(16.0 / 9.0),
            "hp entry matches draw plan"
        );
        let src = include_str!("live.rs");
        let live_fn = src
            .find("fn present_frame_live")
            .expect("live frame exists");
        let live_end = src[live_fn..]
            .find("fn present_frame_probed")
            .map(|i| live_fn + i)
            .unwrap_or(src.len());
        let live_body = &src[live_fn..live_end];
        assert!(live_body.contains("hp_bars_plan"), "live uses hp entry");
        assert!(live_body.contains("write_vertices"), "live builds verts");
        assert!(live_body.contains("encode_overlay"), "live draws overlay");
        assert!(live_body.contains("encode_blit"), "live still blits");
        let blit_at = live_body.find("encode_blit").expect("blit sits");
        let overlay_at = live_body.find("encode_overlay").expect("overlay sits");
        assert!(blit_at < overlay_at, "overlay draws after blit");
    }
}

#[cfg(test)]
mod live_sky_toggle_tests {
    use super::live_sky_should_draw;
    #[test]
    fn sky_off_skips_bind_while_all_on_draws() {
        let all = crate::pass_truth::LookToggles::all_on();
        assert!(live_sky_should_draw(&all), "all-on draws bit-identical");
        let mut off = crate::pass_truth::LookToggles::all_on();
        off.disable("sky");
        assert!(!live_sky_should_draw(&off), "sky off skips bind");
        let mut ghost = crate::pass_truth::LookToggles::all_on();
        ghost.disable("no-such-pass");
        assert!(live_sky_should_draw(&ghost), "unknown keeps draw");
        assert_eq!(live_sky_should_draw(&off), live_sky_should_draw(&off));
        let src = include_str!("live.rs");
        let at = src.find("fn present_frame_live").expect("live sits");
        let end = src[at..]
            .find("fn present_frame_probed")
            .map(|i| at + i)
            .unwrap_or(src.len());
        let body = &src[at..end];
        assert!(body.contains("live_sky_should_draw"), "live consults gate");
        assert!(src.contains("submit_skips_for"), "gate lists skips");
        assert!(body.contains("sky_draw"), "sky bind is gated");
        assert!(body.contains("live_sky_feed_for_frame"), "live reads feed");
    }
}

#[cfg(test)]
mod live_sky_feed_tests {
    use super::live_sky_feed_for_frame;
    #[test]
    fn on_reads_bake_off_writes_nothing() {
        // Caller: `present_frame_live` before `encode_forward`. On reads the
        // bake pair, off or ragged stays `None` so the frame stays identical.
        let (capture, _) = crate::sky::bake_sky(
            &crate::sky::SkyParams::midday(),
            crate::sky::SKY_CAPTURE_W,
            crate::sky::SKY_CAPTURE_H,
        )
        .expect("day-rig bakes");
        let (bake, _) = crate::sky::bake_look_from_day(&capture).expect("look bakes");
        let anchor = crate::arena_day::ARENA_DAY_EXPOSURE_EV100;
        let all = crate::pass_truth::LookToggles::all_on();
        let (horizon, exposure) =
            live_sky_feed_for_frame(&all, &bake, anchor).expect("all-on feeds");
        assert_eq!(horizon, crate::sky::look_fog_color(&bake));
        assert_eq!(
            exposure,
            crate::sky::ev100_to_exposure(anchor).expect("anchor maps")
        );
        let mut off = crate::pass_truth::LookToggles::all_on();
        off.disable("sky");
        assert_eq!(live_sky_feed_for_frame(&off, &bake, anchor), None);
        let mut ragged = bake;
        ragged.texels.truncate(5);
        assert_eq!(live_sky_feed_for_frame(&all, &ragged, anchor), None);
    }
}

#[cfg(test)]
mod live_overlay_capture_tests {
    use crate::present::{DrawItem, DrawList, MeshKey, PresentError, Presenter};
    use crate::test_gpu::{require_or_skip, NoAdapter};

    fn init_or_skip(test: &str) -> Option<Presenter> {
        let probe = match Presenter::init_offscreen() {
            Ok(presenter) => Ok(presenter),
            Err(PresentError::NoAdapter) => {
                Err(NoAdapter("no wgpu adapter on this machine".into()))
            }
            Err(e) => panic!("presenter init failed (must fail loudly): {e}"),
        };
        require_or_skip(test, probe)
    }

    fn cube_list(key: MeshKey, width: u32, height: u32) -> DrawList {
        let scene = crate::v1::default_scene(width, height).expect("default scene is valid");
        DrawList {
            camera: scene.camera,
            light: scene.light,
            items: vec![DrawItem {
                mesh: key,
                material: scene.material,
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            }],
        }
    }

    fn rgba(png: &[u8]) -> Vec<u8> {
        image::load_from_memory(png)
            .expect("capture is a valid PNG")
            .to_rgba8()
            .into_raw()
    }

    /// Pixel box of a 0..1 hud rect through the shared overlay matrix.
    /// Pixel `i` covers centre `i + 0.5`, so the box keeps exactly the pixels
    /// whose centre falls inside the projected rect. Truncating both corners
    /// (`as u32` on each) dropped the far row and column whenever the edge had
    /// a fractional part: the bar's right edge lands at 40.8 px, so the last
    /// covered column (centre 40.5) fell outside a box that ended at 40 and
    /// read as a leak.
    fn rect_px(rect: [f32; 4], aspect: f32, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let m = crate::overlay::overlay_mvp(aspect);
        let to_px = |x: f32, y: f32| {
            let nx = m[0][0] * x + m[3][0];
            let ny = m[1][1] * y + m[3][1];
            (
                (nx + 1.0) * 0.5 * width as f32,
                (-ny + 1.0) * 0.5 * height as f32,
            )
        };
        let (ax, ay) = to_px(rect[0], rect[1]);
        let (bx, by) = to_px(rect[2], rect[3]);
        let (minx, maxx) = (ax.min(bx), ax.max(bx));
        let (miny, maxy) = (ay.min(by), ay.max(by));
        let x0 = (minx - 0.5).ceil().clamp(0.0, width as f32) as u32;
        let y0 = (miny - 0.5).ceil().clamp(0.0, height as f32) as u32;
        let x1 = ((maxx - 0.5).floor() + 1.0).clamp(0.0, width as f32) as u32;
        let y1 = ((maxy - 0.5).floor() + 1.0).clamp(0.0, height as f32) as u32;
        (x0, y0, x1.max(x0), y1.max(y0))
    }

    #[test]
    fn hp_rect_holds_bars_not_scene() {
        // Caller: `render_offscreen` capture path (the arena capture and the
        // headless smoke twin read their PNG from it).
        // Behavior: a staged hp bar changes the pixels inside its screen
        // rect; the frame outside that rect stays byte-identical.
        let Some(mut presenter) = init_or_skip("hp_rect_holds_bars_not_scene") else {
            return;
        };
        let key = MeshKey::new(1, [0xA1; 32]);
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(160, 120).expect("valid size applies");
        let list = cube_list(key, 160, 120);
        let bare = presenter
            .render_offscreen(&list)
            .expect("bare frame renders");
        let bare_png = bare.png.expect("bare frame has pixels");
        let bare_px = rgba(&bare_png);
        presenter.clear_overlay();
        let [x0, y0, x1, y1] = crate::overlay::hp_bar_rect(0);
        assert!(
            presenter.push_hp_bar(x0, y0, x1, y1, 1.0),
            "stage one full hp bar"
        );
        let with = presenter
            .render_offscreen(&list)
            .expect("hud frame renders");
        assert!(
            with.manifest.is_clean(),
            "hud frame stays validation-clean, got {:?}",
            with.manifest
        );
        let with_png = with.png.expect("hud frame has pixels");
        let with_px = rgba(&with_png);
        assert_eq!(bare_px.len(), with_px.len(), "same frame size");
        let (rx0, ry0, rx1, ry1) = rect_px([x0, y0, x1, y1], 160.0 / 120.0, 160, 120);
        let (mut inside, mut outside) = (0usize, 0usize);
        for y in 0..120u32 {
            for x in 0..160u32 {
                let i = ((y * 160 + x) * 4) as usize;
                let moved = (0..3)
                    .any(|c| (i32::from(bare_px[i + c]) - i32::from(with_px[i + c])).abs() > 16);
                if !moved {
                    continue;
                }
                if (rx0..rx1).contains(&x) && (ry0..ry1).contains(&y) {
                    inside += 1;
                } else {
                    outside += 1;
                }
            }
        }
        assert!(
            inside >= 16,
            "the hp rect must hold bar pixels, moved inside={inside}"
        );
        assert_eq!(outside, 0, "the bar draw stays inside its rect");
    }
}

// ---------------------------------------------------------------------------
// Live staging allocation gate: staging one live frame must read the same
// byte images while adding no heap allocation per draw. A `to_bytes()` call
// site in the stager built one `Vec` per draw; the ALLOC regex of
// rust-quality never sees it because it greps call sites for `to_vec`, not
// for the helper that wraps it. Skips (passes) with no adapter.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod live_stage_alloc_tests {
    use super::super::submit::{effective_camera, MATERIAL_STRIDE};
    use crate::present::{
        DrawItem, DrawList, MeshKey, PresentError, Presenter, MAX_DRAWS_PER_FRAME,
    };
    use crate::test_gpu::{require_or_skip, NoAdapter};
    use engine_core::alloc_probe::measure;

    fn init_or_skip(test: &str) -> Option<Presenter> {
        let probe = match Presenter::init_offscreen() {
            Ok(presenter) => Ok(presenter),
            Err(PresentError::NoAdapter) => {
                Err(NoAdapter("no wgpu adapter on this machine".into()))
            }
            Err(e) => panic!("presenter init failed (must fail loudly): {e}"),
        };
        require_or_skip(test, probe)
    }

    /// One live stage, exactly as `present_frame_live` runs it: the frame
    /// camera once, then the shared stager over the frame's draw order.
    /// Returns the staged byte-image lengths (camera, light), so the image
    /// sizes are proven beside the allocation count.
    fn stage(presenter: &mut Presenter, list: &DrawList) -> (usize, usize) {
        let frame = effective_camera(list, 96, 96).expect("frame camera builds");
        let mut order_buf = [0usize; MAX_DRAWS_PER_FRAME];
        let draws = list.items.len().min(MAX_DRAWS_PER_FRAME);
        for (slot, at) in order_buf[..draws].iter_mut().enumerate() {
            *at = slot;
        }
        let (cam, light) = presenter
            .stage_frame_with_camera(list, &frame, frame.built_view_proj, &order_buf[..draws])
            .expect("live stage");
        (cam.len(), light.len())
    }

    #[test]
    fn live_stage_allocs_do_not_grow_with_the_draw_count() {
        // Caller: `stage_frame_with_camera` (the live and probe frames).
        // Behavior: the per-draw material pack adds no heap as draws grow.
        let Some(mut presenter) = init_or_skip("live_stage_allocs_do_not_grow_with_the_draw_count")
        else {
            return;
        };
        let key = MeshKey::new(0x5A11, [0x21; 32]);
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        let scene = crate::v1::default_scene(96, 96).expect("default scene is valid");
        let item = DrawItem {
            mesh: key,
            material: scene.material.clone(),
            base_color: None,
            normal_map: None,
            orm: None,
            emissive: None,
        };
        let list_with = |draws: usize| DrawList {
            camera: scene.camera.clone(),
            light: scene.light.clone(),
            items: vec![item.clone(); draws],
        };
        let (one, many) = (list_with(1), list_with(48));
        // Warmup outside the counting: the steady-state contract.
        let warm = stage(&mut presenter, &many);
        assert_eq!(warm, (80, 32), "camera 80 B plus light 32 B");
        let (one_dims, single) = measure(|| stage(&mut presenter, &one));
        let (many_dims, wide) = measure(|| stage(&mut presenter, &many));
        assert_eq!(one_dims, (80, 32), "one draw stages the same bytes");
        assert_eq!(many_dims, (80, 32), "48 draws stage the same bytes");
        assert_eq!(single.calls, 0, "one live stage allocates nothing");
        assert_eq!(
            wide.calls, single.calls,
            "the live stage allocates nothing more as the draw count grows"
        );
        eprintln!(
            "live stage allocs: 1 draw {} / 48 draws {}",
            single.calls, wide.calls
        );
        assert_eq!(
            presenter.material_scratch.len(),
            48 * MATERIAL_STRIDE,
            "one 256-byte stride per draw lands in the reused scratch"
        );
    }
}
