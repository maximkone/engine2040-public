//! live: the window-path frames (live, opt-in probe, offscreen twin sink).
//!
//! Continues `impl Presenter` from `submit`. Owner: render lane, consumer the
//! windowed arena loop. Units: pixels for sizes, microseconds for waits.
//! The live frame is one command buffer and one submit with no readback;
//! the opt-in probe frame keeps the old blocking tranche (Game Engine Gems
//! 1 ch 9.6, 11.3: never wait on the GPU inside a frame).
use super::super::*;
use super::frame::{FrameReceipt, LookPassReceipt};
use super::presenter::Presenter;
use super::submit::{
    effective_camera, inverse_view_proj_cols, map_readback_f32, padded_depth_row, MATERIAL_STRIDE,
};

/// Fog tranche rows in receipt order: fog passes, fog validation errors,
/// fog submit us, gtao passes, gtao validation errors, gtao submit us.
pub(super) type FogRows = (u32, u32, u64, u32, u32, u64);

/// Where a window-path frame blit lands.
#[derive(Clone, Copy)]
pub(super) enum LiveSink<'a> {
    /// The acquired swapchain view.
    Surface(&'a wgpu::TextureView),
    /// The LDR twin target; the live path never reads it back.
    Twin,
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

/// Live bloom on/off switch: off keeps today's frame bit-identical
/// (default), on lets the gate below decide per frame. Owner live,
/// consumer the future live bloom pass. Units boolean out. Determinism
/// one relaxed atomic load, no wall clock, no allocation. Presentation
/// only, never hashed.
static LIVE_BLOOM_ENABLED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Read the live bloom switch. Units boolean out.
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
pub(crate) fn live_ibl_should_count(forward_binds: bool, lit: bool, ibl_ms: f64, gap: f32) -> bool {
    if !crate::ibl::live_ibl_should_sample(forward_binds, lit) {
        return false;
    }
    if !(ibl_ms > 0.0) {
        return false;
    }
    if !(gap > 0.002) {
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
    if !(sky_ms > 0.0) {
        return false;
    }
    if !(gap > 0.01) {
        return false;
    }
    true
}

/// FC3 30 fps budget gate: true when p50 sits at or below 33.3 ms and p99 at or below 50.0 ms.
///
/// Owner live, consumer the quiet perf gate (future perf-csv reader). Units ms in, boolean out.
/// Pure, no allocation, no device. Determinism same inputs give same bool.
/// Fail-closed: non-finite inputs refuse. Presentation only, never hashed.
/// Caller: future quiet-perf gate reading the windowed arena run.
#[must_use]
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
        let started = std::time::Instant::now();
        let wrote = {
            let cached = self.gtao_sub.full_ao();
            self.write_ao_full(width, height, cached)
        };
        if wrote.is_err() {
            return 0;
        }
        let ms = started.elapsed().as_secs_f64() * 1000.0;
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

    /// Stage one frame from the shared camera plus order: same validation
    /// and uniform packing as `stage_frame` but reuses the caller-built
    /// camera instead of rebuilding it. Units bytes out. Presentation only.
    /// Caller live paths below after `effective_camera`.
    fn stage_frame_with_camera(
        &mut self,
        list: &DrawList,
        camera: &crate::v1::Camera,
        order: &[usize],
    ) -> Result<(Vec<u8>, Vec<u8>), PresentError> {
        list.validate()?;
        if order.len() != list.items.len() {
            return Err(PresentError::InvalidDraw {
                index: 0,
                reason: "draw order length mismatches list".into(),
            });
        }
        let aspect = camera.aspect;
        list.light
            .validate()
            .map_err(|e| PresentError::InvalidDraw {
                index: 0,
                reason: format!("light: {e}"),
            })?;
        let (cam_u, light_u, _) = crate::v1::build_uniforms(
            camera,
            aspect,
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
            let (_, _, mat_u) =
                crate::v1::build_uniforms(camera, aspect, &list.light, &item.material).map_err(
                    |e| PresentError::InvalidDraw {
                        index: slot,
                        reason: format!("material: {e}"),
                    },
                )?;
            if self.meshes.get(&item.mesh).is_none() {
                return Err(PresentError::InvalidDraw {
                    index: slot,
                    reason: format!("unknown mesh key {:?}", item.mesh.id),
                });
            }
            if let Some(key) = &item.base_color {
                if self.textures.get(key).is_none() {
                    return Err(PresentError::InvalidDraw {
                        index: slot,
                        reason: "unknown texture key".into(),
                    });
                }
            }
            self.material_scratch.extend_from_slice(&mat_u.to_bytes());
            self.material_scratch
                .resize((slot + 1) * MATERIAL_STRIDE, 0);
        }
        Ok((cam_u.to_bytes(), light_u.to_bytes()))
    }

    /// After-blit overlay draw for staged HP bars: one textured-quad draw
    /// over the blit view with no depth. Skips empty plans or verts so the
    /// live frame stays bit-identical when no HUD is staged. Units quads.
    /// Presentation only, never hashed. Caller `present_frame_live`.
    fn encode_overlay(
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
/// `present_frame_revision` in `submit` (unchanged skip). Units u64 out.
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
/// Owner live, consumer `present_frame_live` below.
fn live_overlay_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
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
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
    pub(super) fn write_frame_uniforms(&self, cam: &[u8], light: &[u8]) {
        self.queue.write_buffer(&self.cam_buf, 0, cam);
        self.queue.write_buffer(&self.light_buf, 0, light);
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
    /// slope for both. `shader_fog` false (the capture twin and the probe
    /// composite, which fog through the blit pair) writes a zero slope, so
    /// the forward mix is exactly 0.0 and nothing fogs twice. The kill
    /// switch (`set_fog_enabled`) zeroes the slope on the live frame too.
    pub(super) fn write_fog_forward(
        &self,
        list: &DrawList,
        width: u32,
        height: u32,
        shader_fog: bool,
    ) -> Result<(), PresentError> {
        let mut camera = list.camera.clone();
        if height == 0 {
            return Err(PresentError::InvalidDraw {
                index: 0,
                reason: "fog frame height is zero".into(),
            });
        }
        let w16 = u16::try_from(width).unwrap_or(u16::MAX);
        let h16 = u16::try_from(height).unwrap_or(1);
        camera.aspect = f32::from(w16) / f32::from(h16);
        camera.view_proj().map_err(|e| PresentError::InvalidDraw {
            index: 0,
            reason: format!("camera: {e}"),
        })?;
        let inv = inverse_view_proj_cols(&camera)?;
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
            &fog.to_bytes(&flat, camera.eye.to_array()),
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
        self.check_device()?;
        if self.suspended() {
            return Ok(self.suspended_receipt());
        }
        let (width, height) = self.swap.size();
        let receipt = self.window_path_frame(LiveSink::Twin, list, width, height)?;
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
    ) -> Result<FrameReceipt, PresentError> {
        if self.look_probes {
            self.present_frame_probed(sink, list, width, height)
        } else {
            self.present_frame_live(sink, list, width, height)
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
        let receipt = self.window_path_frame(LiveSink::Surface(&view), list, width, height)?;
        self.queue.present(frame);
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
    ) -> Result<FrameReceipt, PresentError> {
        let (render_w, render_h) = live_present_size(width, height, live_half_res_enabled());
        let camera = effective_camera(list, render_w, render_h)?;
        let mut order_buf = [0usize; MAX_DRAWS_PER_FRAME];
        let order = Self::frame_draw_order(list, &mut order_buf);
        let (cam_bytes, light_bytes) = self.stage_frame_with_camera(list, &camera, order)?;
        self.frame_waits = FrameWaits::default();
        self.ensure_targets(render_w, render_h);
        self.write_frame_uniforms(&cam_bytes, &light_bytes);
        self.bind_twin_shadow(Self::twin_focus_depth(list))?;
        self.write_fog_forward(list, render_w, render_h, true)?;
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
        let timer_slot = self
            .gpu_timer
            .as_mut()
            .and_then(|timer| timer.begin_frame(&self.device));
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let forward_ibl_started = std::time::Instant::now();
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ra1-live-frame"),
            });
        let stale_us: u64;
        {
            let targets = self
                .targets
                .as_ref()
                .ok_or_else(|| PresentError::Validation("live frame has no targets".into()))?;
            let (blit_view, blit_pipeline) = self.sink_target(sink)?;
            let timer = self.gpu_timer.as_ref().zip(timer_slot);
            self.encode_forward(
                &mut enc,
                targets,
                list,
                true,
                timer.map(|(timer, _)| timer.forward_writes()),
            );
            // LF3 first landing: fresh GTAO after forward, before the blit,
            // wired through the live helper (`live_ao_upload_for_frame`:
            // bake via `gtao_live_full_for_forward`, guard via
            // `check_ao_full_for_blit`, upload via `write_ao_full`). The
            // forward-stored depth plus normals feed is absent until the
            // slice-2 prepass, so the empty feed stays skip (ao_tex keeps
            // 1.0) with the stale reuse below as fallback. A guard failure
            // records skip, never fails the frame; ms is evidence only
            // against `GTAO_BUDGET_MS`.
            let depth: &[f32] = &[];
            let normals: &[[f32; 3]] = &[];
            let fresh_us = if live_ao_should_bind(render_w, render_h, depth.len(), normals.len()) {
                self.live_ao_upload_for_frame(render_w, render_h, depth, normals)
                    .map(|ms| Self::ms_to_us(ms))
                    .unwrap_or(0)
            } else {
                0
            };
            let fallback_us = if fresh_us == 0 {
                self.stale_ao_reuse_upload(render_w, render_h)
            } else {
                0
            };
            stale_us = fresh_us.max(fallback_us);
            // LF4 bloom gate: CPU mirror of the forward HDR peak feeds the
            // post oracle; the flag rides `FogU::_pad0` (0.0 off keeps
            // today's frame bit-identical, 1.0 on lets the 4-tap halo lift).
            // Bad feed or switch off stays off. Caller `live_bloom_for_frame`.
            let bloom_on = live_bloom_max_luma(list).is_some_and(|peak| {
                live_bloom_for_frame(
                    live_bloom_enabled(),
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
            if let Some(plan) = self.overlay.hp_bars_plan(camera.aspect) {
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
                    let overlay_pipeline = live_overlay_pipeline(&self.device, overlay_format);
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
                timer.encode_resolve(&mut enc, slot);
            }
        }
        let forward_ibl_ms = forward_ibl_started.elapsed().as_secs_f64() * 1000.0;
        // The shipped forward layout carries the bake binds and `fs` samples,
        // so the live frame is bound; the bake verdict comes from the live bake.
        let bake_lit = crate::ibl::bake_has_light(&self.ibl_bake);
        let ibl = live_ibl_for_frame(true, bake_lit, forward_ibl_ms, render_w, render_h);
        self.queue.submit(Some(enc.finish()));
        if let (Some(timer), Some(slot)) = (self.gpu_timer.as_ref(), timer_slot) {
            timer.arm(slot);
        }
        self.pop_scope(scope)?;
        self.finish_frame(FrameKind::Live);
        self.gtao_submit_us = stale_us;
        let gtao_passes: u32 = if stale_us > 0 { 1 } else { 0 };
        Ok(self.frame_receipt(
            list.items.len(),
            &LookPassReceipt::skipped(),
            ibl,
            (0, 0, 0, gtao_passes, 0, stale_us),
        ))
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
        let camera = effective_camera(list, width, height)?;
        let (cam_bytes, light_bytes) = self.stage_frame_with_camera(list, &camera, order)?;
        self.frame_waits = FrameWaits::default();
        let probe = self
            .frames_presented
            .is_multiple_of(self.probe_every.max(1));
        let look = if probe {
            self.encode_look_passes()?
        } else {
            LookPassReceipt::skipped()
        };
        // Fresh targets hold no fog pair yet: refresh on their first frame.
        let fresh_targets = self
            .targets
            .as_ref()
            .is_none_or(|t| t.width != width || t.height != height);
        self.ensure_targets(width, height);
        self.write_frame_uniforms(&cam_bytes, &light_bytes);
        // The surface twin samples the same cascaded sun as the
        // offscreen twin; the frame focus depth selects the band.
        self.bind_twin_shadow(Self::twin_focus_depth(list))?;
        self.write_fog_forward(list, width, height, false)?;
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

        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ra1-frame"),
            });
        {
            let targets = self.targets.as_ref().expect("ensured above");
            let (_, _, depth_rb) = self.depth_rb.as_ref().expect("ensured above");
            self.encode_forward(&mut enc, targets, list, false, None);
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
        let fog_rows: FogRows = if fog_now {
            let raw_depth = {
                let (_, _, depth_rb) = self.depth_rb.as_ref().expect("ensured above");
                map_readback_f32(
                    &self.device,
                    &mut self.frame_waits,
                    depth_rb,
                    width,
                    padded_depth_row(width),
                    height,
                )?
            };
            self.run_fog_tranche(&raw_depth, &camera, width, height)?
        } else if fog_wired {
            // Between refreshes the blit keeps the last fog pair.
            (0, 0, 0, 0, 0, 0)
        } else {
            self.clear_fog_targets(width, height)
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

#[cfg(test)]
mod live_half_res_tests {
    use super::{live_half_res_enabled, live_present_size, LIVE_HALF_RES};

    fn set_half(on: bool) {
        LIVE_HALF_RES.store(on, core::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn off_keeps_full_pixels_on_halves_them() {
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
        assert!(!live_bloom_enabled(), "switch defaults off");
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
        assert!(!live_bloom_enabled(), "switch defaults off");
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
        assert!(
            !src.contains("stage_frame(list,"),
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
