//! submit: frame encode/submit plus free GPU helpers (window build).
//!
//! Continues `impl Presenter` from `presenter`; helpers used across the
//! split are `pub(super)`, everything else keeps its exact shape.
use super::super::*;
use super::debug_gbuffer::{clear_attachment, DebugTargets};
use super::frame::{
    decal_err, ibl_err, layer_err, particle_err, sky_err, trail_err, FrameOutput, FrameReceipt,
    LookPassReceipt, Targets,
};
use super::gate::LiveLook;
use super::live::FogRows;
use super::presenter::Presenter;
use super::submit_cull::{live_submit_order, StagedFrame};
use super::timing::GpuTimer;
use super::{DEPTH_FORMAT, HDR_CLEAR, HDR_FORMAT, OUTPUT_FORMAT, ROW_ALIGN};
use crate::v1::{self, Manifest};
use crate::{material_layer, particles_gpu, sky};
#[cfg(feature = "window")]
use std::sync::Arc;

/// Last sun depth submit wall time in us (receipt evidence slot).
/// Relaxed atomic, presentation only, never hashed.
/// Owner submit, consumer `render_offscreen` receipt plus `sun_shadow` tests.
/// The live encode runs every frame the switch is on (default), so this is
/// the measured EG11 sun depth cost behind `arena per-pass shadow`; it stays
/// 0 only on skip frames (`forward_sun_enabled()` off) and on frames whose
/// sun direction is invalid.
/// Caller: `encode_forward` below stores every live frame.
static SUN_LAST_US: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read the last sun depth submit wall time in us.
/// Units us out. Determinism: wall time, evidence only, never hashed.
/// Caller: `render_offscreen` receipt plus `sun_shadow` receipt tests.
#[must_use]
pub(crate) fn sun_last_us() -> u64 {
    SUN_LAST_US.load(core::sync::atomic::Ordering::Relaxed)
}

/// Remember the last sun depth submit wall time in us.
/// Units us in. Determinism: wall time, evidence only.
fn sun_last_us_store(us: u64) {
    SUN_LAST_US.store(us, core::sync::atomic::Ordering::Relaxed);
}

/// Fill one live frame receipt's sun depth row from the us `encode_forward`
/// stored for this frame's map. The window path builds its receipt through
/// `live::frame_receipt`, which starts that row at 0, so `arena per-pass
/// shadow` read `shadow=0.000ms` on live frames while the depth pass ran.
/// Pure: the caller passes the stored value, so a skipped encode (switch off
/// or a bad sun direction) posts 0 and never a stale reading from the frame
/// before. Capture frames already patch the same row in
/// `render_offscreen_capture`.
/// Units us in/out. Presentation only, never hashed.
/// Caller: `render_surface` below (both presenting arms).
#[must_use]
fn live_shadow_row(mut receipt: FrameReceipt, shadow_us: u64) -> FrameReceipt {
    receipt.shadow_submit_us = shadow_us;
    receipt
}

/// EG11 sun depth switch: on (default) records the live sun depth map inside
/// `encode_forward` before the forward draws, so `arena per-pass shadow`
/// measures the pass the row Accept names; off skips it for A/B evidence.
/// Nothing samples the map yet, so pixels stay bit-identical (EG8 PROBE) and
/// the map under `Presenter::sun_sub` is built once, never per frame.
/// Owner submit, consumer `encode_forward` below.
/// Units boolean out. Determinism one relaxed atomic load, no wall clock,
/// no allocation. Presentation only, never hashed.
static FORWARD_SUN_ENABLED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(true);

/// Read the sun depth switch. Units boolean out.
/// Caller: `encode_forward` below.
#[must_use]
pub(super) fn forward_sun_enabled() -> bool {
    FORWARD_SUN_ENABLED.load(core::sync::atomic::Ordering::Relaxed)
}

/// Flip the sun depth switch and return the value it held (the undo value).
/// Caller: `Presenter::set_pass` (`sun_shadow`), the door's render steer.
/// Units boolean in and out. Presentation only, never hashed.
pub(super) fn set_forward_sun_enabled(on: bool) -> bool {
    FORWARD_SUN_ENABLED.swap(on, core::sync::atomic::Ordering::Relaxed)
}

/// How `encode_forward` draws the sky for one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SkyPlan {
    /// No sky triangle: the clear colour shows where no draw covers.
    Skip,
    /// One far-plane triangle inside the forward pass (the capture twin and
    /// the 4x path, whose resolve happens at the end of that pass).
    InPass,
    /// The triangle in its own pass after forward, with its own device
    /// timestamps, reading the stored depth (the live 1x frame): the sky cost
    /// is measured on its own and never hides inside the forward time.
    OwnPass,
}

/// The sky plan one forward encode follows. An explicit capture decision
/// (`Some`) wins and always draws inside the forward pass; otherwise the frame's
/// own request stands, except that a 4x frame cannot split the pass (the
/// resolve closes it), so an own-pass request draws in the pass there.
/// Pure, no allocation, no device. Caller: `encode_forward` below.
#[must_use]
pub(super) fn resolve_sky_plan(
    capture: Option<bool>,
    requested: SkyPlan,
    multisampled: bool,
) -> SkyPlan {
    match capture {
        Some(true) => SkyPlan::InPass,
        Some(false) => SkyPlan::Skip,
        None if multisampled && requested == SkyPlan::OwnPass => SkyPlan::InPass,
        None => requested,
    }
}

/// Whether `encode_forward` stores depth: when anything reads it after the
/// pass (the own-pass sky, the RL9 fog feed copy on capture and probe
/// frames), else the pass discards it. Pure, no allocation, no device. HDR
/// pixels stay bit-identical either way. Caller: `encode_forward` below.
#[must_use]
pub(super) fn forward_depth_should_store(keep_depth: bool, sky: SkyPlan) -> bool {
    keep_depth || sky == SkyPlan::OwnPass
}

/// What one forward encode does, built by the frame that calls it.
pub(super) struct ForwardLook<'a> {
    /// Draw the list; off leaves the cleared frame (the `graph` row off).
    pub(super) draws: bool,
    /// Record the sun depth map first (the `shadow_cluster` row on).
    pub(super) sun_depth: bool,
    /// The frame's own sky request; a capture decision overrides it.
    pub(super) sky: SkyPlan,
    /// Something reads depth after the pass (fog feed copy, AO).
    pub(super) keep_depth: bool,
    /// Record the staged particle draw into the HDR target (QL12): set on
    /// frames that staged a live batch this frame, so a live frame never
    /// repeats a stale batch.
    pub(super) particles: bool,
    /// Device timestamps around the forward pass.
    pub(super) timestamps: Option<wgpu::RenderPassTimestampWrites<'a>>,
    /// Device timestamps around the own-pass sky.
    pub(super) sky_timestamps: Option<wgpu::RenderPassTimestampWrites<'a>>,
    /// Device timestamps around the particle draw pass.
    pub(super) particle_timestamps: Option<wgpu::RenderPassTimestampWrites<'a>>,
}

impl<'a> ForwardLook<'a> {
    /// The live window frame: the draw, sun and sky switches the frame's
    /// `LiveLook` carries (the sky in its own pass when on), depth kept for the
    /// sky pass and for GTAO (`gtao`), the fight particles when staged (FEEL5,
    /// `fight_fx.rs`), and the device timestamps of the frame's timer slot.
    #[must_use]
    pub(super) fn live(
        look: &LiveLook,
        sky: bool,
        gtao: bool,
        particles: bool,
        timer: Option<&'a GpuTimer>,
    ) -> Self {
        Self {
            draws: look.forward,
            sun_depth: look.sun_depth,
            sky: if sky { SkyPlan::OwnPass } else { SkyPlan::Skip },
            keep_depth: gtao,
            particles,
            timestamps: timer.map(GpuTimer::forward_writes),
            sky_timestamps: timer.map(GpuTimer::sky_writes),
            particle_timestamps: None,
        }
    }

    /// The window probe frame: every pass on with no sky of its own, depth
    /// kept for the fog feed and the AO, the staged particle batch drawn when
    /// `particles` is set (a held frame stages nothing), and the particle
    /// pass bracketed by the frame timer's particle queries.
    #[must_use]
    pub(super) fn probed(particles: bool, timer: Option<&'a GpuTimer>) -> Self {
        Self {
            draws: true,
            sun_depth: true,
            sky: SkyPlan::Skip,
            keep_depth: true,
            particles,
            timestamps: None,
            sky_timestamps: None,
            particle_timestamps: timer.map(GpuTimer::particle_writes),
        }
    }
}

impl ForwardLook<'_> {
    /// A blocking frame (capture twin, probe composite): every pass on, no
    /// sky of its own (a capture decision may add it), depth kept for the fog
    /// feed, no timestamps; fight particles when staged (stepped per tick, so twins repeat).
    #[must_use]
    pub(super) fn blocking(particles: bool) -> Self {
        Self {
            draws: true,
            sun_depth: true,
            sky: SkyPlan::Skip,
            keep_depth: true,
            particles,
            timestamps: None,
            sky_timestamps: None,
            particle_timestamps: None,
        }
    }
}

/// Capture sky arm: no explicit decision, the frame's own plan decides.
const CAPTURE_SKY_LIVE: u8 = 0;
/// Capture sky arm: the capture twin skips the sky triangle.
const CAPTURE_SKY_OFF: u8 = 1;
/// Capture sky arm: the capture twin draws the sky triangle.
const CAPTURE_SKY_ON: u8 = 2;

thread_local! {
    /// EG8 capture sky decision for the frame `encode_forward` is encoding:
    /// `None` on live and probe frames (the frame's own plan decides),
    /// `Some(draw)` while `render_offscreen_with_toggles` renders the arena
    /// capture twin with an explicit look-toggle set. The twin keeps its depth
    /// under every arm, so the RL9 fog feed keeps its scaffold while the sky
    /// row can really be switched on and off for A/B frames.
    /// Units tri-state out. Determinism: one thread-local cell read, no
    /// clock, no allocation. Presentation only, never hashed. The cell is
    /// per thread: the decision lives for one capture call and
    /// `encode_forward` reads it on that same thread, so a capture on one
    /// thread never flips the sky of a live frame encoding on another.
    /// Caller: `encode_forward` below (sky and depth decision).
    static CAPTURE_SKY: core::cell::Cell<u8> = const { core::cell::Cell::new(CAPTURE_SKY_LIVE) };
}

/// Encode one capture sky decision as its stored state; pure, no IO.
#[must_use]
fn capture_sky_state(decision: Option<bool>) -> u8 {
    match decision {
        Some(true) => CAPTURE_SKY_ON,
        Some(false) => CAPTURE_SKY_OFF,
        None => CAPTURE_SKY_LIVE,
    }
}

/// Decode one stored capture sky state; pure, no IO. Anything that is not
/// one of the two named explicit arms stays the live arm.
#[must_use]
fn capture_sky_from_state(state: u8) -> Option<bool> {
    match state {
        CAPTURE_SKY_OFF => Some(false),
        CAPTURE_SKY_ON => Some(true),
        _ => None,
    }
}

/// Read the capture sky decision; `None` means the frame's own plan decides.
/// Units tri-state out. Determinism: one thread-local cell read.
/// Caller: `encode_forward` below.
#[must_use]
fn capture_sky_decision() -> Option<bool> {
    capture_sky_from_state(CAPTURE_SKY.with(core::cell::Cell::get))
}

/// Store the capture sky decision and hand back the previous one, so the
/// caller puts the live arm back even when the frame fails.
/// Units tri-state in and out. Determinism: one thread-local cell swap.
/// Caller: `render_offscreen_with_toggles` below.
fn set_capture_sky(decision: Option<bool>) -> Option<bool> {
    capture_sky_from_state(CAPTURE_SKY.with(|cell| cell.replace(capture_sky_state(decision))))
}

/// Forward color target count: HDR, the R32Uint id target, and the LF5
/// motion target. Units count out. Determinism fixed layout, never hashed.
/// Caller: `ensure_targets` below plus `forward_id_attachment_tests`.
#[must_use]
pub(super) fn forward_color_target_count() -> u32 {
    3
}

/// Forward color target formats in attachment order: HDR, id, motion.
/// Units formats out. Determinism fixed layout, never hashed.
/// Caller: `ensure_targets` below plus `forward_id_attachment_tests`.
#[must_use]
pub(super) fn forward_color_target_formats() -> [wgpu::TextureFormat; 3] {
    [
        HDR_FORMAT,
        super::frame::ID_TARGET_FORMAT,
        super::MOTION_FORMAT,
    ]
}

impl Presenter {
    /// Per-frame torch bytes beside the light bytes, zeroed for clears.
    /// Stack image, allocates nothing. Reuses the `forward_bind` owner.
    /// Caller: `stage_frame` below stages the same bytes into `torch_buf`;
    /// tests in `present/tests_gpu.rs` pin the byte layout.
    ///
    /// Test-only: the live path stages the scene's own list through
    /// [`Presenter::torch_bytes_from_scene`], so this one exists for the
    /// byte-layout pins and would be dead code in a release build.
    #[cfg(test)]
    pub(crate) fn torch_bytes(
        &self,
        list: &DrawList,
    ) -> [u8; super::forward_bind::TORCH_BLOCK_BYTES] {
        let frame = super::forward_bind::TorchFrame::for_draws(!list.items.is_empty());
        let mut out = [0u8; super::forward_bind::TORCH_BLOCK_BYTES];
        frame.pack_into(&mut out);
        out
    }

    /// Per-frame torch bytes from the scene's torch list (VS7): the
    /// placement actors become lights. A scene that feeds none keeps the
    /// arena's built-in pair. Refuses a broken list loudly.
    pub(crate) fn torch_bytes_from_scene(
        &self,
        list: &DrawList,
    ) -> Result<[u8; super::forward_bind::TORCH_BLOCK_BYTES], PresentError> {
        let frame =
            super::forward_bind::TorchFrame::from_scene(!list.items.is_empty(), &self.torch_lights)
                .map_err(|e| PresentError::InvalidDraw {
                    index: 0,
                    reason: format!("vs7 torch lights: {e}"),
                })?;
        let mut out = [0u8; super::forward_bind::TORCH_BLOCK_BYTES];
        frame.pack_into(&mut out);
        Ok(out)
    }
}

impl Presenter {
    /// Record the Tier A look tranche on the device: RL4 sky, RL6 layer
    /// chart, particle pool and decal pool, all under the shared look
    /// bind, before the forward PBR draws. The particle pool steps one
    /// presentation tick per call; every pass reports 0 validation
    /// errors or the frame fails loudly with the culprit named. Runs on
    /// live sizes only: suspended callers never reach here. The IBL
    /// chart is not part of this tranche: it samples the init-time bake
    /// with no forward input, so it submits after the forward draws
    /// (see `submit_ibl_chart`) and the receipt counts it separately.
    pub(super) fn encode_look_passes(
        &mut self,
        frame: &FrameCamera,
    ) -> Result<LookPassReceipt, PresentError> {
        // Each submitter below times its own submit plus readback wait, and a
        // queue still holding the last frame's draws makes the first of them
        // read that frame's GPU time as its own (2026-10-24: sky=16 ms for a
        // few-KB copy). Drain once so every probe row times only itself. A
        // probe frame blocks by design, so the drain costs nothing extra.
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        self.frame_waits.waits = self.frame_waits.waits.saturating_add(1);
        let frame_uniform = self.look_frame();
        let cam = self.look_cam();
        // RL8: the RA1 skybox plus the RL2 ambient read the same bound
        // bake every frame the RL4 submit binds below (RL7 fog follows in
        // the fog tranche off the same bake). A corrupt bake fails the
        // frame loudly here instead of tinting it.
        let sky_sample = self.skybox_sample(frame_uniform.sun_dir)?;
        let ambient = self.ibl_ambient();
        if !sky_sample.iter().all(|c| c.is_finite()) || !ambient.iter().all(|c| c.is_finite()) {
            return Err(PresentError::Validation(
                "rl8 look bake is not finite".into(),
            ));
        }
        let sky = sky::submit_gpu_sky(
            &self.device,
            &self.queue,
            &frame_uniform,
            &self.sky_capture,
            &mut self.sky_sub,
        )
        .map_err(sky_err)?;
        let layers = material_layer::submit_chart_gpu(
            &self.device,
            &self.queue,
            &frame_uniform,
            &mut self.layer_sub,
        )
        .map_err(layer_err)?;
        self.particles.step(particles_gpu::TICK_DT);
        self.set_vfx_camera(frame);
        // QL12: the pool's live set is staged as flipbook instances under the
        // frame camera; the frame records the draw into its HDR pass
        // (`encode_forward`), so the particle row is a device pass instead of
        // a bind probe.
        let particle = self
            .particle_sub
            .stage_pool(
                &self.device,
                &self.queue,
                &self.particles,
                frame.built_view_proj.to_cols_array_2d(),
                frame.camera_right(),
                frame.camera_up(),
            )
            .map_err(particle_err)?;
        // LF14: the frame's sword ribbon. The game replaces it on every staged
        // frame; an empty list clears the last batch, so an idle frame stages
        // no strip. The camera is the same one the forward draws with.
        self.trail_sub
            .stage(
                &self.device,
                &self.queue,
                &self.trail_verts,
                &self.trail_lens,
                frame.built_view_proj.to_cols_array_2d(),
                frame.camera_right(),
                frame.camera_up(),
            )
            .map_err(trail_err)?;
        let decal_pool = &self.decal_pool;
        let decal_receipt = decal_pool
            .submit_gpu(
                &self.device,
                &self.queue,
                &frame_uniform,
                &cam,
                &mut self.decal_sub,
            )
            .map_err(decal_err)?;
        // Each submitter above is one submit, one map and one poll wait; the
        // particle stage is CPU-only (the frame's own error scope owns its
        // validation) and reports its device ms through the frame timer.
        self.frame_waits.map_and_wait_n(3);
        Ok(LookPassReceipt {
            passes: 4,
            validation_errors: sky.validation_errors
                + layers.validation_errors
                + decal_receipt.validation_errors,
            sky_ms: sky.submit_ms,
            layer_ms: layers.submit_ms,
            particle_ms: self.particle_device_ms(particle.submit_ms),
            decal_ms: decal_receipt.submit_ms,
        })
    }

    /// The particle row's ms: the device timestamp of the newest resolved
    /// particle pass when the frame has a timer, the CPU stage ms otherwise
    /// (a capture twin with no timer). Evidence only, never a hash input.
    fn particle_device_ms(&self, stage_ms: f64) -> f64 {
        self.gpu_timer
            .as_ref()
            .map_or(stage_ms, |timer| f64::from(timer.latest_particle_ms()))
    }

    /// Submit the IBL material chart after the forward draws: the
    /// init-time bake sampled on the device (prefiltered cubemap,
    /// irradiance SH, BRDF LUT), the same pass the graph declares ahead
    /// of forward. Separated from the look tranche because the chart
    /// feeds no forward input; the receipt counts it on its own rows.
    /// Returns (passes, validation errors, submit us) for the receipt.
    /// Fails loudly with the culprit named; runs on live sizes only.
    pub(super) fn submit_ibl_chart(&mut self) -> Result<(u32, u32, u64), PresentError> {
        let frame = self.look_frame();
        let bake = &self.ibl_bake;
        let chart = bake
            .submit_gpu(&self.device, &self.queue, &frame, &mut self.ibl_sub)
            .map_err(ibl_err)?;
        self.frame_waits.map_and_wait();
        // LF3 first landing: fresh GTAO bind sequenced in the post-forward
        // slot via `live_gtao_ao` (gate plus half plus bilateral upsample
        // plus guard). The forward-stored feed is absent until the slice-2
        // prepass, so the gate stays skip and the IBL receipt below keeps
        // today's rows; ms is evidence only.
        let _lf3_ao_ms: Option<f64> = {
            let depth: &[f32] = &[];
            let normals: &[[f32; 3]] = &[];
            let (w, h) = self.swap.size();
            Self::live_gtao_ao(w, h, depth, normals)
        };
        Ok((1, chart.validation_errors, Self::ms_to_us(chart.submit_ms)))
    }

    /// Live GTAO bind for one forward-stored depth feed: gate, half-res oracle
    /// plus the bilateral upsample, then the blit guard on the full-res field.
    /// Pure CPU bind, no device, no map, no poll; empty or bad feed returns
    /// `None` with no allocation so the caller keeps `ao_tex` at 1.0, never
    /// fails the frame. Ms out is evidence only. Presentation only, never hashed.
    /// Owner submit, consumer `submit_ibl_chart` in the post-forward slot
    /// (which runs after `encode_forward` before `encode_blit` on both the
    /// capture twin and the probe frame). Presentation only, never hashed.
    /// Caller: `submit_ibl_chart` above.
    pub(super) fn live_gtao_ao(
        width: u32,
        height: u32,
        depth: &[f32],
        normals: &[[f32; 3]],
    ) -> Option<f64> {
        // Gate first: no alloc on skip, mirroring the live fresh path.
        if !super::live::live_ao_should_bind(width, height, depth.len(), normals.len()) {
            return None;
        }
        let cfg = Self::gtao_config_for_frame(width, height).ok()?;
        let (field, ms) = crate::gtao::gtao_live_full_for_forward(depth, normals, &cfg).ok()?;
        crate::gtao::check_ao_full_for_blit(&field, width, height).ok()?;
        Some(ms)
    }

    /// Live sun depth pre-step: kept for the capture order; the live map now
    /// runs inside `encode_forward` before the forward draws, so this step
    /// records zero and lets the forward pass post the real shadow time.
    /// Caller: `render_offscreen` below.
    pub(super) fn encode_sun_depth(
        &self,
        _enc: &mut wgpu::CommandEncoder,
        _list: &DrawList,
    ) -> Result<(), PresentError> {
        sun_last_us_store(0);
        Ok(())
    }

    pub(super) fn ensure_targets(&mut self, width: u32, height: u32) {
        let fresh = match &self.targets {
            Some(t) => t.width != width || t.height != height,
            None => true,
        };
        if !fresh {
            return;
        }
        // SP63 id slice stays byte-identical: HDR plus id pair the
        // forward pass carries; cold path only, never per frame. LF5 adds
        // the motion target as attachment 2.
        let target_formats = forward_color_target_formats();
        debug_assert_eq!(
            forward_color_target_count(),
            3,
            "forward carries HDR, id and motion"
        );
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let hdr = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ra1-hdr"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: target_formats[0],
            // COPY_SRC: the linear frame can be read back for evidence.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let out_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ra1-output"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OUTPUT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        // E11: the look-table target. `TEXTURE_BINDING` lets a later frame's
        // pass read it, `COPY_SRC` the capture; the blit never writes it.
        let lut_out_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ra1-look-out"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OUTPUT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ra1-depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            // COPY_SRC feeds the RL9 per-frame depth readback; TEXTURE_BINDING
            // lets the live GTAO kernels load the stored depth.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        // RL9 fog-factor pair for the blit composite (R32 float each).
        // Zero-filled at creation: factor 0 (no tint) until the first
        // fog tranche uploads; AO is opened below.
        let fog_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ra1-fog-factor"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let ao_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ra1-fog-ao"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            // COPY_SRC lets the device tests copy this field back and prove the
            // blit darkens the base; STORAGE_BINDING lets the live GTAO
            // upsample write it on the device.
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        self.creates.buffer();
        self.creates.bind_group();
        let fog_col_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ra1-fog-color"),
            size: super::super::FOG_BLIT_UNIFORM_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hdr_view = hdr.create_view(&wgpu::TextureViewDescriptor::default());
        let fog_view = fog_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let ao_view = ao_texture.create_view(&wgpu::TextureViewDescriptor::default());
        // LF3 landing 2: the forward pass reads the frame's occlusion field
        // from group 1. The field is a target texture, so the bind is
        // rebuilt with the targets; steady frames create no bind groups.
        let ao_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ra1-ao-field-bind"),
            layout: &self.ao_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&ao_view),
            }],
        });
        // RL36: the dual-filter bloom mips over the HDR view, sized with the
        // targets (one texture per rung, one uniform per dispatch). The blit's
        // halo slot samples rung 0; a frame whose bloom gate is closed never
        // reads it.
        let bloom = self.bloom_gpu.sized(
            &self.device,
            &self.queue,
            &hdr_view,
            width,
            height,
            &self.bloom.config,
        );
        // The blit bind group is rebuilt with the targets (not per frame),
        // so steady-state frames allocate no bind groups or pipelines.
        let blit_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ra1-blit-bind"),
            layout: &self.blit_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.blit_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&fog_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&ao_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: fog_col_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&bloom.halo_view),
                },
            ],
        });
        // AO starts open (1.0) so pre-fog frames show the clear forward
        // frame; the fog tranche overwrites both textures per frame.
        // Cold path only (target rebuild), never per frame.
        let texels = width as usize * height as usize;
        let mut open = vec![0u8; texels * 4];
        let one = 1.0f32.to_le_bytes();
        for chunk in open.chunks_exact_mut(4) {
            chunk.copy_from_slice(&one);
        }
        let copy_layout = wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &ao_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &open,
            copy_layout,
            size,
        );
        // The pair stays neutral (factor 0, AO open) on live frames, but
        // the blit's texel clamp needs the frame size in range: write it
        // once with the targets, not per frame.
        let neutral = super::super::fog_blit_uniform_bytes([0.0, 0.0, 0.0], width, height);
        self.queue.write_buffer(&fog_col_buf, 0, &neutral);
        // SP63 draw-id target: one u32 per pixel, cold starts clear so the
        // shader slice (next) has a known background. 1x path keeps `msaa`
        // at `None`. Cold path only.
        let id_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ra1-id"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: target_formats[1],
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let id_view = id_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let id_clear = super::frame::id_clear_bytes(width, height);
        if !id_clear.is_empty() {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &id_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &id_clear,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                size,
            );
        }
        // EG13 on-path 4x pair from the plan; cold path only, inside the
        // transient cap. Off keeps `None` (today's 1x path byte-identical).
        debug_assert!(
            super::msaa::msaa_transient_within_cap(width, height, &self.msaa),
            "MSAA transient over budget"
        );
        // LF5 motion: one 1x Rg16Float target, cleared by the forward pass
        // every frame. COPY_SRC lets a test read the field back; TEXTURE_BINDING
        // lets the LF5c resolve reproject with it. No 4x twin exists (that
        // would push the 4x transient over its cap), so the MSAA path clears
        // this one instead of writing it.
        let motion_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ra1-motion"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: target_formats[2],
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let motion_view = motion_texture.create_view(&wgpu::TextureViewDescriptor::default());
        // LF5c: the resolve's history pair, its two resolve binds and the two
        // blit binds over the resolved views ride the targets. Cold path only
        // (target rebuild): the stale history is dropped here, so the first
        // frame after a resize follows the current image.
        self.taa.ensure(
            &self.device,
            &hdr_view,
            &motion_view,
            width,
            height,
            super::taa_resolve::BlitFeed {
                layout: &self.blit_layout,
                sampler: &self.blit_sampler,
                fog: &fog_view,
                ao: &ao_view,
                bloom: &bloom.halo_view,
                fog_col: &fog_col_buf,
            },
        );
        let msaa = if super::msaa::msaa_enabled(&self.msaa) {
            let plan = super::msaa::attachment_plan(&self.msaa);
            let msaa_color = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("ra1-hdr-msaa"),
                size,
                mip_level_count: 1,
                sample_count: plan.color_samples,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let msaa_depth = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("ra1-depth-msaa"),
                size,
                mip_level_count: 1,
                sample_count: plan.depth_samples,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let color_view = msaa_color.create_view(&wgpu::TextureViewDescriptor::default());
            let depth_view = msaa_depth.create_view(&wgpu::TextureViewDescriptor::default());
            Some(super::frame::MsaaTargets {
                samples: plan.color_samples,
                color: msaa_color,
                color_view,
                depth: msaa_depth,
                depth_view,
            })
        } else {
            None
        };
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
        // The sprite and ribbon passes fade against this depth (group 1):
        // rebuilt with the targets, never per frame.
        self.particle_sub
            .bind_depth(crate::vfx_soft::vfx_depth_bind(&self.device, &depth_view));
        self.trail_sub
            .bind_depth(crate::vfx_soft::vfx_depth_bind(&self.device, &depth_view));
        self.creates.bind_group();
        self.creates.bind_group();
        let gtao = self
            .gtao_gpu
            .sized(&self.device, &depth_view, &ao_view, width, height);
        self.targets = Some(Targets {
            width,
            height,
            hdr_view,
            #[cfg(test)]
            hdr_texture: hdr,
            out_view: out_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            out_texture,
            lut_out_view: lut_out_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            lut_out_texture,
            depth_view,
            depth_texture,
            fog_texture,
            ao_texture,
            ao_bind,
            gtao,
            bloom,
            id_texture,
            id_view,
            id_clear,
            motion_view,
            motion_texture,
            fog_col_buf,
            blit_bind,
            msaa,
        });
        // Fresh targets start neutral (zero-init fog texture, AO opened
        // and neutral uniform written above), so the fog-off branches
        // below need no reset upload until a tranche dirties the pair.
        self.fog_pair_dirty = false;
        // SW150: the outline chain sizes with the frame, beside these targets.
        self.ensure_outline(width, height);
    }

    /// Pick each draw's forward bind group for the frame: its resident
    /// texture's group (built when the texture uploaded), or the 1x1 white
    /// fallback for `None` draws (identity multiply). A pure lookup into
    /// scratch the presenter keeps: no bind group is created here. Staged
    /// lists already proved every key resident, so a missing group fails
    /// with `InvalidDraw`, never a caller panic; callers run this after
    /// `stage_frame`.
    pub(super) fn resolve_draw_binds(&mut self, list: &DrawList) -> Result<(), PresentError> {
        // One resident-list scan per distinct texture this frame; every
        // repeat is answered by the stack memo in `bind_memo`.
        let binds = &self.texture_binds;
        super::bind_memo::resolve_bind_choices(
            list.items.iter().map(|item| item.base_color.as_ref()),
            &mut self.draw_binds,
            |key| Self::bind_index_of(binds, key),
        )
        .map(|_scans| ())
        .map_err(|index| PresentError::InvalidDraw {
            index,
            reason: "unknown texture key".into(),
        })?;
        self.pick_map_sets(list)
    }

    /// VR2: a draw that names a normal map picks its material set (a few
    /// sets at most, so a short scan of `map_sets`, not of the resident
    /// binds; draws with no map cost one push).
    fn pick_map_sets(&mut self, list: &DrawList) -> Result<(), PresentError> {
        let sets = &self.map_sets;
        self.draw_sets.clear();
        for (index, item) in list.items.iter().enumerate() {
            let choice = match &item.normal_map {
                None => None,
                Some(key) => {
                    Some(sets.iter().position(|set| set.key == *key).ok_or_else(|| {
                        PresentError::InvalidDraw {
                            index,
                            reason: "unknown normal map key (no material set uploaded)".into(),
                        }
                    })?)
                }
            };
            self.draw_sets.push(choice);
        }
        Ok(())
    }

    /// Index of `key` in the resident bind list. The one scan of that list;
    /// only a memo miss reaches it.
    fn bind_index_of(binds: &[(TextureKey, wgpu::BindGroup)], key: &TextureKey) -> Option<usize> {
        binds.iter().position(|(known, _)| known == key)
    }

    /// Bind group chosen for draw `draw` by `resolve_draw_binds`.
    fn draw_bind(&self, draw: usize) -> Option<&wgpu::BindGroup> {
        if let Some(Some(set)) = self.draw_sets.get(draw) {
            return self.map_sets.get(*set).map(|set| &set.bind);
        }
        match self.draw_binds.get(draw)? {
            Some(index) => self.texture_binds.get(*index).map(|(_, bind)| bind),
            None => Some(&self.fallback_bind),
        }
    }

    /// Every named texture slot of one draw must be resident: an unknown key
    /// fails the frame with `InvalidDraw` naming the draw index, exactly like
    /// `base_color`. `None` slots bind the fallbacks, so untextured draws
    /// stay byte-identical. The normal slot is checked by `pick_map_sets`: a
    /// normal key with no uploaded material set fails there.
    pub(super) fn require_draw_textures(
        &self,
        item: &DrawItem,
        index: usize,
    ) -> Result<(), PresentError> {
        for (reason, key) in [
            ("unknown base color texture key", &item.base_color),
            ("unknown orm texture key", &item.orm),
            ("unknown emissive texture key", &item.emissive),
        ] {
            if let Some(key) = key {
                if self.textures.get(key).is_none() {
                    return Err(PresentError::InvalidDraw {
                        index,
                        reason: reason.into(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Stage one frame from the shared frame camera: validate the list and
    /// light, resolve every mesh key, build the culled submit order, and pack
    /// the uniform byte images from the frame's built view-projection.
    /// Returns the order (pipeline-grouped, outside-frustum draws dropped)
    /// plus stack camera (80 B) and light (32 B) arrays; the per-draw
    /// materials land in `material_scratch`, one 256-byte stride each, ready
    /// for the ring write. Steady frames allocate nothing. Fails before any
    /// GPU work.
    pub(super) fn stage_frame(
        &mut self,
        list: &DrawList,
        frame: &FrameCamera,
    ) -> Result<StagedFrame, PresentError> {
        list.validate()?;
        list.light
            .validate()
            .map_err(|e| PresentError::InvalidDraw {
                index: 0,
                reason: format!("light: {e}"), // alloc-ok: cold validation failure, never steady-state
            })?;
        // EG38: the live list is built here, so the tested sphere cull runs
        // here too; the forward and depth passes draw this order.
        let bounds = self.frame_draw_bounds(list);
        let (order, submitted) = live_submit_order(list, &bounds, frame.built_view_proj);
        self.submitted_draws = submitted;
        // Camera plus light bytes come from a fixed valid material: the
        // packer ignores it for those uniforms, and every real material
        // already passed `DrawList::validate` above.
        let (cam_u, light_u, _) = v1::build_uniforms_with_view_proj(
            &frame.camera,
            frame.built_view_proj,
            &list.light,
            &v1::PbrMaterial {
                base_color: [0.5, 0.5, 0.5],
                metallic: 0.0,
                roughness: 0.5,
                exposure: 1.0,
            },
        )
        .map_err(|e| PresentError::InvalidDraw {
            index: 0,
            reason: format!("frame: {e}"), // alloc-ok: cold uniform-build failure, never steady-state
        })?;
        self.material_scratch.clear();
        for (index, item) in list.items.iter().enumerate() {
            let kind = self.material_kind(item.mesh);
            let param = self.material_param(item.mesh, kind);
            let mat_u =
                v1::build_material(&item.material).map_err(|e| PresentError::InvalidDraw {
                    index,
                    reason: format!("material: {e}"), // alloc-ok: cold material failure, never steady-state
                })?;
            if self.meshes.get(&item.mesh).is_none() {
                return Err(PresentError::InvalidDraw {
                    index,
                    reason: format!("unknown mesh key {:?}", item.mesh.id), // alloc-ok: cold missing-mesh refusal, never steady-state
                });
            }
            // Texture residency: unknown keys in any named slot fail with
            // their draw index, mirroring meshes, before any GPU work is
            // encoded.
            self.require_draw_textures(item, index)?;
            self.material_scratch
                .extend_from_slice(&mat_u.to_array_with_kind(kind, param));
            self.material_scratch
                .resize((index + 1) * MATERIAL_STRIDE, 0);
        }
        // Torch bytes beside the light bytes; zeroed for clears so clears
        // stay clean. Staged into `torch_buf` for the forward bind. The
        // scene's torch list (VS7) rides here when the scene feeds one.
        let torch = self.torch_bytes_from_scene(list)?;
        self.queue.write_buffer(&self.torch_buf, 0, &torch);
        Ok(StagedFrame {
            cam_bytes: cam_u.to_array(),
            light_bytes: light_u.to_array(),
            order,
            submitted,
        })
    }

    /// Twin sun-shadow bind: resolve the frame focus depth into the
    /// twin sun band and refresh the twin shadow uniform the window
    /// mesh forward reads at group 0 binding 3 under
    /// `shaders/forward.wgsl` (the `fs` entry). One 32-byte sun uniform
    /// write plus one 128-byte contact feet write per live frame;
    /// steady-state frames allocate nothing.
    /// A non-finite depth fails the frame loudly, never a silent band.
    pub(super) fn bind_twin_shadow(&self, view_depth_m: f32) -> Result<(), PresentError> {
        let fail = |e: crate::twin_shadow::TwinShadowError| PresentError::InvalidDraw {
            index: 0,
            reason: format!("rl19 twin shadow: {e}"),
        };
        let band = crate::twin_shadow::twin_active_cascade(&self.twin_sun, view_depth_m)
            .map_err(fail)?
            .min(crate::quality::max_cascade_band(&self.quality));
        self.queue.write_buffer(
            &self.twin_shadow_buf,
            0,
            &crate::twin_shadow::twin_shadow_uniform(&self.twin_sun, band),
        );
        // The live contact feet ride behind the 32 sun bytes (zero until fed).
        self.queue.write_buffer(
            &self.twin_shadow_buf,
            super::contact_live::CONTACT_FEET_OFFSET,
            &self.contact_feet,
        );
        Ok(())
    }

    /// Frame focus depth in meters: eye-to-target distance, the cascade
    /// the frame subject lives in. Non-finite cameras fail loudly at the
    /// bind, never a silent band.
    pub(super) fn twin_focus_depth(list: &DrawList) -> f32 {
        (list.camera.eye - list.camera.target).length()
    }

    /// LF3 landing 2: the depth prepass. It draws the frame's submitted order
    /// with the depth-only pipelines so the GTAO kernel can read the frame's
    /// depth before the forward pass shades it. The forward pass then clears
    /// the depth and writes it again, so the frame's final depth is the one
    /// the forward pass wrote, exactly as before this landing.
    pub(super) fn encode_depth_prepass(
        &self,
        enc: &mut wgpu::CommandEncoder,
        targets: &Targets,
        list: &DrawList,
        order: &[usize],
    ) {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ra1-depth-prepass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &targets.depth_view,
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
        for &draw in order {
            let item = &list.items[draw];
            let (Some(mesh), Some(bind)) = (self.meshes.get(&item.mesh), self.draw_bind(draw))
            else {
                continue;
            };
            let skin_block = self.skin_blocks.get(&item.mesh).copied();
            rp.set_pipeline(if skin_block.is_some() {
                &self.depth_skinned_pipeline
            } else {
                &self.depth_pipeline
            });
            let offset = u32::try_from(draw * MATERIAL_STRIDE).unwrap_or(0);
            let joints = skin_block.map_or(0, skin_block_offset);
            rp.set_bind_group(0, bind, &[offset, joints, joints]);
            rp.set_vertex_buffer(0, mesh.vertex.slice(..));
            rp.set_index_buffer(mesh.index.slice(..), wgpu::IndexFormat::Uint32);
            rp.draw_indexed(0..mesh.count, 0, 0..1);
        }
    }

    /// Forward pass: clear, draw the frame's submitted order, then the sky at
    /// the far plane so the depth test rejects every covered pixel, inside the
    /// pass or in its own pass per the [`SkyPlan`]. `look` carries the frame's
    /// pass switches and the device timestamps; `order` is the culled,
    /// pipeline-grouped list the stage step built. Returns true when the sky
    /// ran in its own timed pass, so the caller resolves the sky timestamps.
    pub(super) fn encode_forward(
        &self,
        enc: &mut wgpu::CommandEncoder,
        targets: &Targets,
        list: &DrawList,
        look: ForwardLook<'_>,
        order: &[usize],
        debug: Option<&DebugTargets>,
    ) -> bool {
        // EG11b: the live sun map runs here; a skipped pass writes `enabled` 0.
        // A3: a debug capture shades nothing but the G-buffer, so it never
        // records the sun map (the live frame's stays as it was).
        let wanted = debug.is_none() && look.sun_depth && forward_sun_enabled();
        sun_last_us_store(self.sun_sub.record(self, enc, list, wanted));
        // EG13: on plus built resolves 4x into 1x HDR, else today's 1x path.
        let (_, should_resolve) =
            super::msaa::msaa_forward_choice(&self.msaa, targets.msaa.is_some());
        let msaa_pair = debug
            .is_none()
            .then(|| targets.msaa.as_ref().filter(|_| should_resolve))
            .flatten();
        let (color_view, depth_view, pipeline, skinned_pipeline, resolve) = match debug {
            Some(dbg) => (
                &dbg.normal_view,
                &dbg.depth_view,
                &dbg.pipeline,
                &dbg.skinned,
                None,
            ),
            None => match msaa_pair {
                Some(pair) => (
                    &pair.color_view,
                    &pair.depth_view,
                    &self.forward_hdr_msaa,
                    &self.forward_skinned_msaa,
                    Some(&targets.hdr_view),
                ),
                None => (
                    &targets.hdr_view,
                    &targets.depth_view,
                    &self.forward_hdr,
                    &self.forward_skinned,
                    None,
                ),
            },
        };
        // LF5: the MSAA pass keeps its single HDR attachment, so it cannot
        // write motion (a 4x twin would break the transient cap). Clear the
        // 1x target once per MSAA frame so a reader sees zero, never the
        // last 1x frame's field.
        if msaa_pair.is_some() {
            let _clear = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ra1-motion-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.motion_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        // Depth is stored while something reads it after the pass: the
        // own-pass sky, the fog feed copy, or a staged splat field (FEEL5).
        // HDR pixels stay bit-identical either way.
        let sky_plan = if debug.is_some() {
            SkyPlan::Skip
        } else {
            resolve_sky_plan(capture_sky_decision(), look.sky, msaa_pair.is_some())
        };
        // The sprite and ribbon passes read the stored depth to fade softly
        // against bodies and the floor; the 4x path keeps no 1x depth, so
        // they draw without the fade there (see `vfx_soft`).
        let vfx_reads_depth = msaa_pair.is_none()
            && ((look.particles && self.particle_sub.staged_count() > 0)
                || self.trail_sub.has_draw());
        let keep_depth = debug.is_some()
            || look.keep_depth
            || vfx_reads_depth
            || (self.decal_sub.has_splat_draw() && msaa_pair.is_none());
        let depth_store = if debug.is_some() || forward_depth_should_store(keep_depth, sky_plan) {
            wgpu::StoreOp::Store
        } else {
            wgpu::StoreOp::Discard
        };
        // SP63: the 1x pass writes the id attachment beside HDR; the MSAA path
        // keeps its single resolved HDR attachment (a 1x id cannot pair).
        let hdr_attachment = wgpu::RenderPassColorAttachment {
            view: color_view,
            depth_slice: None,
            resolve_target: resolve,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(HDR_CLEAR),
                store: wgpu::StoreOp::Store,
            },
        };
        let id_attachment = wgpu::RenderPassColorAttachment {
            view: &targets.id_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        };
        // LF5: the motion target clears every frame on the 1x path; the sky
        // pass below loads it, so geometry keeps its move under the sky.
        let motion_attachment = wgpu::RenderPassColorAttachment {
            view: &targets.motion_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        };
        let color_attachments = [
            Some(hdr_attachment),
            Some(id_attachment),
            Some(motion_attachment),
        ];
        // A3: the debug capture clears its own three channels and writes
        // nothing else, so the frame's HDR, id and motion targets are never
        // touched by a debug render.
        let debug_attachments = debug.map(|dbg| {
            [
                Some(clear_attachment(
                    &dbg.normal_view,
                    wgpu::Color {
                        r: 0.5,
                        g: 0.5,
                        b: 1.0,
                        a: 1.0,
                    },
                )),
                Some(clear_attachment(&dbg.albedo_view, wgpu::Color::TRANSPARENT)),
                Some(clear_attachment(&dbg.ids_view, wgpu::Color::TRANSPARENT)),
            ]
        });
        let color_attachments: &[Option<wgpu::RenderPassColorAttachment<'_>>] =
            match (&debug_attachments, msaa_pair) {
                (Some(attachments), _) => attachments,
                (None, Some(_)) => &color_attachments[..1],
                (None, None) => &color_attachments[..3],
            };
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ra1-forward"),
            color_attachments,
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: depth_store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: look.timestamps,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(pipeline);
        // LF3 landing 2: the frame's occlusion field (group 1). The forward
        // pass scales its ambient diffuse by it; the field belongs to the
        // frame, so one bind serves every draw.
        rp.set_bind_group(1, &targets.ao_bind, &[]);
        // The skinned pipeline binds only while a skinned group draws: a
        // mesh uploaded with a skin block carries 80-byte vertices and reads
        // its joint block through the dynamic offset at binding 7.
        let mut skinned_bound = false;
        // RL32: sorted groups over the staged index list; one bind plus
        // one draw serves each distinct (pipeline, material, mesh) group.
        // Duplicates share buffers plus material bytes, so one draw is
        // byte-equal to N (depth Less drops exact overdraw, blend REPLACE
        // is opaque); distinct meshes stay one draw each until RL60
        // instancing lands.
        // The `graph` row off draws nothing: the pass still clears, so the
        // frame is the empty arena the sky and the post chain show.
        let order: &[usize] = if look.draws { order } else { &[] };
        let mut i = 0;
        while i < order.len() {
            let draw = order[i];
            let item = &list.items[draw];
            let key = rl32_group_key(item);
            let mut j = i + 1;
            while j < order.len() && rl32_group_key(&list.items[order[j]]) == key {
                j += 1;
            }
            let (Some(mesh), Some(bind)) = (self.meshes.get(&item.mesh), self.draw_bind(draw))
            else {
                i = j;
                continue;
            };
            // Material `draw` sits at one 256-byte stride in the ring; the
            // group shares material bytes, so the head slot serves it.
            let offset = u32::try_from(draw * MATERIAL_STRIDE).unwrap_or(0);
            let skin_block = self.skin_blocks.get(&item.mesh).copied();
            if skin_block.is_some() != skinned_bound {
                skinned_bound = skin_block.is_some();
                rp.set_pipeline(if skinned_bound {
                    skinned_pipeline
                } else {
                    pipeline
                });
            }
            let joints = skin_block.map_or(0, skin_block_offset);
            // LF5: binding 22 is the previous joint palette, at the same
            // block offset as binding 7.
            rp.set_bind_group(0, bind, &[offset, joints, joints]);
            rp.set_vertex_buffer(0, mesh.vertex.slice(..));
            // LF5: the static 1x pipeline reads the previous positions from
            // a compact stream; a skinned mesh has no such stream and takes
            // its previous pose from the palette above.
            if let Some(prev) = &mesh.prev_vertex {
                rp.set_vertex_buffer(1, prev.slice(..));
            }
            rp.set_index_buffer(mesh.index.slice(..), wgpu::IndexFormat::Uint32);
            // SP63: the fragment `id` output is the vertex stage's flat
            // `draw_id`, which reads `instance_index`; one instance
            // starting at `draw` makes every covered pixel read back as
            // this draw's list index (the group head). HDR pixels are
            // unchanged: geometry and shading do not read the instance.
            let draw_u = u32::try_from(draw).unwrap_or(0);
            rp.draw_indexed(0..mesh.count, 0, draw_u..draw_u + 1);
            i = j;
        }
        // A3: a debug capture stops after the geometry. The sky, the floor
        // blood, the particles and the trail would paint channels the frame
        // has no business showing.
        if debug.is_some() {
            return false;
        }
        // RL64: the staged impostor crowd records into this pass, after the
        // meshes and before the sky; the pass shape picks the pipeline.
        if look.draws {
            let shape = if msaa_pair.is_some() {
                crate::impostor_gpu::PassShape::ForwardMsaa
            } else {
                crate::impostor_gpu::PassShape::Forward1x
            };
            self.impostor_sub.encode_draw(&mut rp, shape);
        }
        if sky_plan == SkyPlan::InPass {
            // The sky entries read only the fog uniform; any forward bind
            // group carries it (the dynamic material offset is unused).
            // The 4x on path needs the 4x sky pipeline to match the 4x
            // attachments; the 1x path keeps today's pipeline.
            let sky_pipeline = if msaa_pair.is_some() {
                &self.sky_msaa
            } else {
                &self.sky_pipeline
            };
            rp.set_pipeline(sky_pipeline);
            rp.set_bind_group(0, &self.fallback_bind, &[0, 0, 0]);
            rp.draw(0..3, 0..1);
        }
        drop(rp);
        let sky_timed = if sky_plan == SkyPlan::OwnPass {
            self.encode_sky_pass(enc, targets, look.sky_timestamps)
        } else {
            false
        };
        // FEEL5: the floor blood, over the lit frame and under the sparks.
        if msaa_pair.is_none() {
            self.decal_sub
                .encode_splat_pass(enc, &targets.hdr_view, &targets.depth_view);
        }
        // QL12: the staged particle batch composites over the lit frame in one
        // HDR load pass, so the quads land in the picture the blit and the
        // capture read. A frame that staged no batch records nothing.
        if look.particles && self.particle_sub.staged_count() > 0 {
            self.particle_sub
                .set_depth_valid(&self.queue, msaa_pair.is_none());
            self.encode_particle_pass(enc, targets, look.particle_timestamps);
        }
        // LF14: the sword ribbon composites in its own HDR load pass after the
        // sparks; a frame that staged no strip records nothing.
        if self.trail_sub.has_draw() {
            self.trail_sub
                .set_depth_valid(&self.queue, msaa_pair.is_none());
            self.encode_trail_pass(enc, targets);
        }
        // SW150: the locked target's screen-space x-ray outline over the depth
        // this frame just wrote, so a fighter behind the sand berm still reads as
        // a dim warm edge. A frame with no target, the potato tier, or a debug
        // capture records nothing.
        if debug.is_none() {
            self.encode_outline(enc, targets, list);
        }
        sky_timed
    }

    /// The staged sword ribbon as its own render pass after forward: loads the
    /// HDR target and draws one triangle strip per busy slot, with no depth
    /// attachment (the pipeline's declared shape) and no timer slot of its
    /// own. Caller: `encode_forward` above.
    fn encode_trail_pass(&self, enc: &mut wgpu::CommandEncoder, targets: &Targets) {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lf14-trail"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &targets.hdr_view,
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
        self.trail_sub.encode_draw(&mut rp);
    }

    /// The staged particle batch as its own render pass after forward: loads
    /// the HDR target, draws the flipbook quads with no depth attachment (the
    /// pipeline's declared shape) and brackets the pass with its own device
    /// timestamps when the frame has a timer, so the particle cost is a
    /// device number of its own. Caller: `encode_forward` above.
    fn encode_particle_pass(
        &self,
        enc: &mut wgpu::CommandEncoder,
        targets: &Targets,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ra1-particles"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &targets.hdr_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: timestamps,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.particle_sub.encode_draw(&mut rp);
    }

    /// The sky as its own render pass after forward: loads the HDR and id
    /// targets, tests against the stored depth read-only (nothing is
    /// written), and brackets the pass with its own device timestamps, so
    /// the sky cost is a number of its own. Returns true when the pass wrote
    /// timestamps. Caller: `encode_forward` above (1x live frames).
    fn encode_sky_pass(
        &self,
        enc: &mut wgpu::CommandEncoder,
        targets: &Targets,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) -> bool {
        fn keep(view: &wgpu::TextureView) -> Option<wgpu::RenderPassColorAttachment<'_>> {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })
        }
        let timed = timestamps.is_some();
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ra1-sky"),
            color_attachments: &[
                keep(&targets.hdr_view),
                keep(&targets.id_view),
                // LF5: load the geometry's move; sky pixels write zero.
                keep(&targets.motion_view),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &targets.depth_view,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: timestamps,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(&self.sky_pipeline);
        rp.set_bind_group(0, &self.fallback_bind, &[0, 0, 0]);
        rp.draw(0..3, 0..1);
        timed
    }

    pub(super) fn encode_blit(
        &self,
        enc: &mut wgpu::CommandEncoder,
        bind: &wgpu::BindGroup,
        out_view: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ra1-blit"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: out_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.03,
                        g: 0.035,
                        b: 0.06,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: timestamps,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(pipeline);
        rp.set_bind_group(0, bind, &[]);
        rp.draw(0..3, 0..1);
    }

    /// Close a validation scope. Natively the answer is ready at once and a validation error fails
    /// the frame. A browser answers `popErrorScope` from its event loop, which a frame cannot wait
    /// for, so there the answer goes to the scope ledger and the frame succeeds; the error shows up
    /// in [`Presenter::deferred_validation_errors`] a frame or two later.
    pub(super) fn pop_scope(&mut self, scope: wgpu::ErrorScopeGuard) -> Result<(), PresentError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(err) = pollster::block_on(scope.pop()) {
                return Err(PresentError::Validation(format!("{err:?}")));
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let answer = scope.pop();
            self.scopes.defer(Box::pin(async move {
                answer.await.map(|err| format!("{err:?}"))
            }));
        }
        Ok(())
    }

    pub(super) fn suspended_receipt(&self) -> FrameReceipt {
        let (width, height) = self.swap.size();
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
            width,
            height,
            draws: 0,
            validation_errors: 0,
            generation: self.swap.generation(),
            presented: false,
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
            look_passes: 0,
            look_validation_errors: 0,
            ibl_passes: 0,
            ibl_validation_errors: 0,
            fog_passes: 0,
            fog_validation_errors: 0,
            fog_submit_us: 0,
            gtao_passes: 0,
            gtao_validation_errors: 0,
            gtao_submit_us: 0,
            sky_submit_us: 0,
            layer_submit_us: 0,
            particle_submit_us: 0,
            decal_submit_us: 0,
            ibl_submit_us: 0,
            shadow_submit_us: 0,
        }
    }

    fn suspended_output(&self) -> FrameOutput {
        let (width, height) = self.swap.size();
        let manifest = Manifest::new(&self.adapter_name, &self.adapter_driver, width, height, 0);
        FrameOutput {
            png: None,
            manifest,
            receipt: self.suspended_receipt(),
        }
    }

    /// Render one draw list offscreen into the LDR twin and return PNG
    /// bytes plus manifest and receipt: the arena capture frame with every
    /// look pass the twin draws. Same bytes as
    /// [`Self::render_offscreen_with_toggles`] under the all-on set.
    /// Caller: the arena capture harness (`engine-game`) plus the offscreen
    /// frame tests.
    pub fn render_offscreen(&mut self, list: &DrawList) -> Result<FrameOutput, PresentError> {
        self.render_offscreen_with_toggles(list, &crate::pass_truth::LookToggles::all_on())
    }

    /// Render one draw list offscreen under an explicit look pass set, so a
    /// disabled row really skips its pass. The sky row is the pass this
    /// path switches (the live frame path switches more, see
    /// `render_live_offscreen_with_toggles`); the twin keeps storing depth
    /// under every arm so the RL9 fog feed keeps its copy. Everything else is
    /// [`Self::render_offscreen_capture`] verbatim.
    /// Units toggles in, PNG bytes out. Presentation only, never hashed.
    /// Caller: `render_offscreen` above (all-on arm) plus the capture
    /// harness that renders the off frame of the pair.
    pub fn render_offscreen_with_toggles(
        &mut self,
        list: &DrawList,
        toggles: &crate::pass_truth::LookToggles,
    ) -> Result<FrameOutput, PresentError> {
        let previous = set_capture_sky(Some(super::live::live_sky_should_draw(toggles)));
        let out = self.render_offscreen_capture(list);
        set_capture_sky(previous);
        out
    }

    /// Render one draw list offscreen into the LDR twin and return PNG
    /// bytes plus manifest and receipt. While suspended (minimized) no GPU
    /// work runs and `png` is `None`; the last-good pipeline is kept.
    /// This is the explicit capture path: it records the four look passes
    /// (sky, layers, particles, decals) under one shared look bind, then
    /// the IBL chart plus the GTAO plus fog tranche (chart from the
    /// init-time bake, occlusion plus height fog plus contact from this
    /// frame's depth feed) whose pair the blit composites into the twin,
    /// and reads the pixels back. It blocks on the GPU by design, so its
    /// waits are tallied as capture frames, never as live frames.
    fn render_offscreen_capture(&mut self, list: &DrawList) -> Result<FrameOutput, PresentError> {
        self.check_device()?;
        if self.suspended() {
            return Ok(self.suspended_output());
        }
        // The walk stays on load-warmed hits (`get`/`poll_ready`
        // only); a debug miss means a variant was added without warming
        // it at load.
        debug_assert!(self.pipeline_walk_ready());
        debug_assert_eq!(MAX_FRAME_DIM, v1::MAX_CAPTURE_DIM);
        let (width, height) = self.swap.size();
        // One frame camera for the whole capture frame: staging, the fog
        // forward uniform and the RL9 fog inverse all read its built
        // view-projection instead of building the matrix again.
        let frame = effective_camera(list, width, height)?;
        let staged = self.stage_frame(list, &frame)?;
        let (cam_bytes, light_bytes) = (staged.cam_bytes, staged.light_bytes);
        self.frame_waits = FrameWaits::default();
        let look = self.encode_look_passes(&frame)?;
        self.ensure_targets(width, height);
        self.write_frame_uniforms(&cam_bytes, &light_bytes, true);
        // The window twin forward path samples the landed cascaded
        // sun; the frame focus depth selects the active cascade.
        self.bind_twin_shadow(Self::twin_focus_depth(list))?;
        self.write_fog_forward(&frame, false)?;
        // RL9: the fog graph wiring compiles once per shape (steady-state
        // frames reuse it) and the depth staging sizes with it. Frames
        // that cannot fit the fog transients render neutral instead of
        // failing.
        let fog_wired = self.fog_enabled && super::super::fog_fits_frame(width, height);
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
        // Sun depth is owned by the EG11 lane; record zero time so the
        // forward pass runs while that map type lands.
        self.encode_sun_depth(&mut enc, list)?;
        // LF3 landing 2: the capture twin runs the same occlusion field as
        // the live frame, so the capture and the window agree.
        let gtao_run = self.gtao_live_plan(true, width, height);
        let particles = self.stage_fight_fx(&frame)?; // FEEL5: the capture shows them too
        self.stage_fight_splats(&frame)?; // FEEL5: and the blood on the floor
        {
            let targets = self.targets.as_ref().expect("ensured above");
            let (_, _, depth_rb) = self.depth_rb.as_ref().expect("ensured above");
            self.encode_ao_field(
                &mut enc,
                targets,
                list,
                &frame,
                gtao_run,
                None,
                staged.order(),
            )?;
            self.encode_forward(
                &mut enc,
                targets,
                list,
                ForwardLook::blocking(particles),
                staged.order(),
                None,
            );
            // RL9 depth-readback seam: this frame's depth feeds the fog
            // tranche below (same queue order, so the copy sees the
            // forward pass writes).
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
        self.queue.submit(Some(enc.finish()));
        // RL9 fog tranche: decode this frame's depth, run the fog plus
        // contact passes on the device, upload the composite pair. The
        // decode reuses the caller-owned scratch (resized on shape
        // change only) and is handed back after the tranche below.
        let mut raw_depth = std::mem::take(&mut self.depth_decode);
        {
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
        let fog_rows: FogRows = if fog_wired {
            let rows = self.run_fog_tranche(&raw_depth, &frame, width, height);
            self.depth_decode = raw_depth;
            rows?
        } else if self.fog_pair_dirty {
            self.depth_decode = raw_depth;
            self.clear_fog_targets(width, height)
        } else {
            // Pair already neutral (fresh targets start neutral, and the
            // last tranche was already reset): skip the two full-res
            // uploads plus the uniform write this frame.
            self.depth_decode = raw_depth;
            (0, 0, 0, 0, 0, 0)
        };
        // Post-forward chart: same bake, same submitter, own receipt
        // rows, so the look tranche counts exactly the four forward
        // inputs while the chart still submits on every live frame.
        let ibl = self.submit_ibl_chart()?;
        let bytes_per_row = width * 4;
        let padded = bytes_per_row.div_ceil(ROW_ALIGN) * ROW_ALIGN;
        self.creates.buffer();
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ra1-readback"),
            size: (padded as u64) * (height as u64),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        // Second submit: the blit composites the fog pair uploaded above.
        let mut blit_enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ra1-blit-frame"),
            });
        {
            let targets = self.targets.as_ref().expect("ensured above");
            // LF5c: resolve the frame the forward submit wrote, before the
            // blit composites it; off records nothing and reads `hdr_view`.
            self.taa.encode(&self.queue, &mut blit_enc, width, height);
            self.encode_blit(
                &mut blit_enc,
                self.blit_bind_for(targets),
                &targets.out_view,
                &self.twin_pipeline,
                None,
            );
            // EG14: the staged HUD ring draws after the blit into the same
            // LDR view the readback copies, so the arena capture and the
            // smoke twin show the hp bars. An empty ring stays a no-op, so a
            // capture with no HUD staged is bit-identical to today.
            let overlay_aspect = frame.aspect;
            if let Some(plan) = self.overlay.hp_bars_plan(overlay_aspect) {
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
                    let overlay_pipeline = self.overlay_pipeline_for(OUTPUT_FORMAT);
                    self.encode_overlay(
                        &mut blit_enc,
                        Some(plan),
                        &verts[..count],
                        &mvp_bytes,
                        &targets.out_view,
                        &overlay_pipeline,
                    );
                }
            }
            // The debug line pass rides the same after-blit view, one draw and
            // no depth; the off switch or an empty batch keeps the frame equal.
            self.encode_debug_lines(
                &mut blit_enc,
                frame.aspect,
                OUTPUT_FORMAT,
                &targets.out_view,
            );
            // E11: the look pass sits between the graded frame (plus the HUD
            // and lines that ride its view) and the display write. A frame
            // with no table bound records nothing and copies the blit's own
            // bytes, so its pixels stay exactly what they were.
            let display_texture = if self.look_lut_bound() {
                if let Some(bind) =
                    self.lut_pass
                        .bind(&self.device, &self.lut, &targets.out_view, width, height)
                {
                    let pipeline = self.lut_pass.pipeline_for(&self.device, OUTPUT_FORMAT);
                    self.lut_pass
                        .encode(&mut blit_enc, &bind, &pipeline, &targets.lut_out_view);
                }
                &targets.lut_out_texture
            } else {
                &targets.out_texture
            };
            blit_enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: display_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded),
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
        self.queue.submit(Some(blit_enc.finish()));
        let rgba = map_readback(
            &self.device,
            &mut self.frame_waits,
            &readback,
            bytes_per_row,
            padded,
            height,
        )?;
        self.pop_scope(scope)?;
        let png = v1::encode_png_rgba8(&rgba, width, height)
            .map_err(|e| PresentError::Readback(format!("png: {e}")))?; // alloc-ok: cold png-encode failure, never steady-state
        let manifest = Manifest::new(&self.adapter_name, &self.adapter_driver, width, height, 0);
        self.frames_presented += 1;
        self.finish_frame(FrameKind::Capture);
        let mut receipt = self.frame_receipt(list.items.len(), &look, ibl, fog_rows);
        receipt.shadow_submit_us = sun_last_us();
        Ok(FrameOutput {
            png: Some(png),
            manifest,
            receipt,
        })
    }

    /// Render one draw list to the attached surface and present it. Maps
    /// every acquire outcome to its recovery: `Outdated`/`Lost`
    /// reconfigure and ask for a retry, `Timeout`/`Occluded` skip the
    /// frame, validation failures keep last-good and never present.
    /// EG11: both presenting arms post the live sun depth row through
    /// `live_shadow_row`, so the window frame reports the pass
    /// `encode_forward` recorded instead of the receipt builder's 0; the
    /// skipping arms keep the suspended receipt's 0.
    pub fn render_surface(&mut self, list: &DrawList) -> Result<FrameReceipt, PresentError> {
        self.check_device()?;
        if !self.swap.is_live() {
            return Ok(self.suspended_receipt());
        }
        if self.surface.is_none() {
            return Err(PresentError::NoSurface);
        }
        // The acquire owns its frame, so the surface borrow ends here and
        // the arms below may reconfigure (mutable) freely. Its wall time
        // is kept apart: a vsync-bound present mode parks the CPU here.
        let acquire_started = engine_clock::now();
        let current = self
            .surface
            .as_ref()
            .expect("checked above")
            .get_current_texture();
        self.last_acquire_us =
            u64::try_from(engine_clock::since(acquire_started).as_micros()).unwrap_or(u64::MAX);
        self.acquire_us = self.acquire_us.saturating_add(self.last_acquire_us);
        self.acquire_frames = self.acquire_frames.saturating_add(1);
        let (width, height) = self.swap.size();
        match current {
            wgpu::CurrentSurfaceTexture::Success(frame) => {
                let receipt = self.present_frame(frame, list, width, height)?;
                Ok(live_shadow_row(receipt, sun_last_us()))
            }
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                let receipt = self.present_frame(frame, list, width, height)?;
                self.reconfigure_surface();
                Ok(live_shadow_row(receipt, sun_last_us()))
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.reconfigure_surface();
                Err(PresentError::SurfaceOutdated {
                    generation: self.swap.generation(),
                })
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.reconfigure_surface();
                Err(PresentError::SurfaceLost {
                    generation: self.swap.generation(),
                })
            }
            wgpu::CurrentSurfaceTexture::Timeout => Err(PresentError::SurfaceTimeout),
            wgpu::CurrentSurfaceTexture::Occluded => Err(PresentError::SurfaceOccluded),
            wgpu::CurrentSurfaceTexture::Validation => {
                Err(PresentError::Validation("surface acquire failed".into()))
            }
        }
    }

    fn reconfigure_surface(&mut self) {
        // Reconfigure with the stored (nonzero, last-good) size; the
        // pipeline is untouched, so last-good survives the loss.
        if let (Some(surface), Some(config)) = (self.surface.as_ref(), self.surface_config.clone())
        {
            surface.configure(&self.device, &config);
            self.surface_config = Some(config);
        }
    }
}

pub(super) fn uniform_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Bytes of one material uniform.
pub(super) const MATERIAL_BYTES: u64 = 32;
/// Camera uniform struct packs to the 80-byte WGSL image with one pad word.
const _: () = assert!(core::mem::size_of::<v1::CameraUniform>() == 76);
/// Light uniform struct packs to the 32-byte WGSL image with one pad word.
const _: () = assert!(core::mem::size_of::<v1::LightUniform>() == 28);
/// Material uniform struct packs to the 32-byte WGSL image with two pad words.
const _: () = assert!(core::mem::size_of::<v1::MaterialUniform>() == 24);
/// Stride between two draws' materials in the ring: the uniform offset
/// alignment every device grants (256), so any draw index is a legal
/// dynamic offset.
pub(super) const MATERIAL_STRIDE: usize = 256;
/// Material ring length in bytes: one stride per allowed draw.
pub(super) const MATERIAL_RING_LEN: usize = 256 * 256;
/// Material ring size for the buffer descriptor.
pub(super) const MATERIAL_RING_BYTES: u64 = 256 * 256;
const _: () = assert!(
    MAX_DRAWS_PER_FRAME <= 256,
    "the ring holds one stride per draw"
);

/// Layout entry of the material uniform: a dynamic offset into the ring.
pub(super) fn material_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: wgpu::BufferSize::new(MATERIAL_BYTES),
        },
        count: None,
    }
}

/// Joints one skin block holds: the length of the `skin_joints` array that
/// `vs_skinned` indexes in `shaders/forward.wgsl`.
pub(crate) const SKIN_BLOCK_JOINTS: usize = 64;
/// Bytes of one skin block: 64 matrices of 64 bytes. A multiple of 256, so
/// every block start is a legal dynamic storage offset on any device.
pub(super) const SKIN_BLOCK_BYTES: u64 = 4096;
/// [`SKIN_BLOCK_BYTES`] as the `u32` a dynamic offset takes.
pub(super) const SKIN_BLOCK_STRIDE: u32 = 4096;
/// Skin blocks the joint buffer holds: one per skinned body in the frame.
pub(crate) const SKIN_BLOCKS: usize = 16;
/// Bytes of the `skin_params` uniform (`SkinParams` in the shader).
pub(super) const SKIN_PARAMS_BYTES: u64 = 16;
/// Skinned vertex stride in bytes: pos(12) + normal(12) + uv(8) +
/// tangent(16) + joints(4 x u32, 16) + weights(4 x f32, 16). Matches `SIn`
/// in the shader; the tangent carries the mesh frame with handedness in w
/// (EG12 slice c), so a normal map on a skinned body is not distorted.
pub(super) const SKINNED_VERTEX_STRIDE: u64 = 80;
/// Byte offset of the tangent inside one skinned vertex.
pub(super) const SKINNED_TANGENT_OFFSET: u64 = 32;
const _: () = assert!(SKIN_BLOCK_JOINTS * 64 == 4096);
const _: () = assert!(SKIN_BLOCK_STRIDE.is_multiple_of(256));
const _: () = assert!(SKIN_BLOCKS >= 8, "eight fighters each get a block");

/// Layout entry of the joint palette: one block of the read-only storage
/// buffer, picked per draw by a dynamic offset (binding 7, vertex stage).
pub(super) fn skin_joints_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: true,
            min_binding_size: wgpu::BufferSize::new(SKIN_BLOCK_BYTES),
        },
        count: None,
    }
}

/// Layout entry of the `skin_params` uniform (binding 8, vertex stage).
pub(super) fn skin_params_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(SKIN_PARAMS_BYTES),
        },
        count: None,
    }
}

/// Dynamic offset of skin block `block` in the joint buffer. Pure.
#[must_use]
pub(super) fn skin_block_offset(block: u8) -> u32 {
    u32::from(block) * SKIN_BLOCK_STRIDE
}

/// RL32 sort key over one draw: (pipeline, material, mesh, depth, stable).
/// Pipeline is the shared compiled-structure key (lobes 0 plus slot mask);
/// depth is 0 for opaque today (no per-draw depth until RL60 carries a
/// transform); stable is the input index, so the order is total without
/// relying on a stable sort. Pure, no alloc. Caller: `rl32_submit_order`.
pub(super) fn rl32_sort_key(item: &DrawItem, stable: u64) -> (u64, u64, u64, u32, u64) {
    let slots = item.texture_slots();
    let mask = (u8::from(slots[0].is_some())) | (u8::from(slots[2].is_some()) << 2);
    let pipeline = crate::material_pipeline::pipeline_key_for(0, mask);
    (pipeline, item.material_id(), item.mesh.id, 0, stable)
}

/// RL32 group key: one bind plus one draw serves each distinct triple.
/// Depth and stable id order inside the sort but never split a group.
/// Pure, no alloc. Caller: `encode_forward` plus tests.
pub(super) fn rl32_group_key(item: &DrawItem) -> (u64, u64, u64) {
    let (pipeline, material, mesh, _, _) = rl32_sort_key(item, 0);
    (pipeline, material, mesh)
}

/// RL32 index order over caller `buf`: sorted groups, no heap past setup.
/// The stack `order_buf` in `encode_forward` is the only storage; the sort
/// is unstable but total through the stable lane, so same input gives the
/// same order twice. Caller: `encode_forward`.
pub(super) fn rl32_submit_order<'a>(
    list: &DrawList,
    buf: &'a mut [usize; MAX_DRAWS_PER_FRAME],
) -> &'a [usize] {
    let n = list.items.len().min(MAX_DRAWS_PER_FRAME);
    for (i, s) in buf[..n].iter_mut().enumerate() {
        *s = i;
    }
    buf[..n].sort_unstable_by_key(|&d| {
        rl32_sort_key(&list.items[d], u64::try_from(d).unwrap_or(u64::MAX))
    });
    &buf[..n]
}

pub(super) fn uniform_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// One frame's effective camera plus the view-projection built once for
/// it. One build owner: [`effective_camera`]; the uniform packer, the fog
/// forward uniform and the RL9 depth decode all read `built_view_proj`
/// instead of building the matrix again. `Deref` hands out the camera
/// itself, so a consumer that only needs `eye`/`target`/`aspect` reads it
/// as a plain [`v1::Camera`].
pub(super) struct FrameCamera {
    /// Draw-list camera with the pass aspect already applied.
    camera: v1::Camera,
    /// The frame's built view-projection: the camera above went through
    /// `Camera::view_proj` once and nothing rebuilds it.
    pub(super) built_view_proj: glam::Mat4,
}

impl std::ops::Deref for FrameCamera {
    type Target = v1::Camera;

    fn deref(&self) -> &v1::Camera {
        &self.camera
    }
}

impl FrameCamera {
    /// The frame's built view-projection, in the shape [`v1::Camera`]'s own
    /// builder returns it. [`effective_camera`] already validated the camera,
    /// so this never fails and never rebuilds.
    pub(super) fn view_proj(&self) -> Result<glam::Mat4, v1::CameraError> {
        Ok(self.built_view_proj)
    }

    /// World-space camera right axis (unit length), the axis the particle
    /// billboards expand along. Derived from the frame camera's own eye,
    /// target and up: `right = normalize(forward x up)`.
    pub(super) fn camera_right(&self) -> [f32; 3] {
        let forward = (self.camera.target - self.camera.eye).normalize_or_zero();
        let right = forward.cross(self.camera.up).normalize_or_zero();
        right.to_array()
    }

    /// World-space camera up axis (unit length), the billboards' vertical
    /// axis: `up = normalize(right x forward)`, re-orthogonalized against
    /// the frame camera's forward so a tilted up never skews a quad.
    pub(super) fn camera_up(&self) -> [f32; 3] {
        let forward = (self.camera.target - self.camera.eye).normalize_or_zero();
        let right = forward.cross(self.camera.up).normalize_or_zero();
        right.cross(forward).normalize_or_zero().to_array()
    }
}

/// Effective frame camera: the draw-list camera with the aspect the
/// forward pass renders at, plus the view-projection built once for it.
/// One writer: the live and capture frames both go through here so the
/// decode can never disagree with the frame that produced the depth.
pub(super) fn effective_camera(
    list: &DrawList,
    width: u32,
    height: u32,
) -> Result<FrameCamera, PresentError> {
    let mut camera = list.camera.clone();
    camera.aspect = width as f32 / height as f32;
    let built_view_proj = camera.view_proj().map_err(|e| PresentError::InvalidDraw {
        index: 0,
        reason: format!("camera: {e}"),
    })?;
    Ok(FrameCamera {
        camera,
        built_view_proj,
    })
}

/// Inverse view-projection in column-major `to_cols_array` order (the
/// same image the camera uniform carries) for the RL9 depth decode.
/// Inverts the frame's built matrix; it never builds one of its own.
pub(super) fn inverse_view_proj_cols(frame: &FrameCamera) -> Result<[[f32; 4]; 4], PresentError> {
    let view_proj = frame.view_proj().map_err(|e| PresentError::InvalidDraw {
        index: 0,
        reason: format!("fog camera: {e}"),
    })?;
    let flat = view_proj.inverse().to_cols_array();
    if !flat.iter().all(|c| c.is_finite()) {
        return Err(PresentError::Readback(
            "rl9 camera inverse is not finite".into(),
        ));
    }
    Ok([
        [flat[0], flat[1], flat[2], flat[3]],
        [flat[4], flat[5], flat[6], flat[7]],
        [flat[8], flat[9], flat[10], flat[11]],
        [flat[12], flat[13], flat[14], flat[15]],
    ])
}

/// 256-byte aligned depth row stride for `copy_texture_to_buffer`.
pub(super) fn padded_depth_row(width: u32) -> u32 {
    (width * 4).div_ceil(ROW_ALIGN) * ROW_ALIGN
}

/// Map a padded depth staging buffer into raw f32 texels, dropping the
/// row padding. Mirrors `map_readback` for the color twin. The caller
/// owns `out`: it is resized on shape change only and reused every fog
/// frame, so steady-state fog frames allocate nothing here.
pub(super) fn map_readback_f32(
    device: &wgpu::Device,
    waits: &mut FrameWaits,
    buffer: &wgpu::Buffer,
    width: u32,
    padded: u32,
    height: u32,
    out: &mut Vec<f32>,
) -> Result<(), PresentError> {
    let fail = |what: &str| PresentError::Readback(format!("rl9 depth {what}"));
    waits.map_and_wait();
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| fail(&format!("poll: {e}")))?;
    rx.recv()
        .map_err(|e| fail(&format!("map channel: {e}")))?
        .map_err(|e| fail(&format!("map: {e}")))?;
    let w = width as usize;
    let n = w * height as usize;
    if out.len() != n {
        out.resize(n, 0.0);
    }
    {
        let mapped = slice
            .get_mapped_range()
            .map_err(|e| fail(&format!("mapped range: {e}")))?;
        for (y, row) in out.chunks_exact_mut(w).enumerate() {
            let off = y * padded as usize;
            for (dst, src) in row.iter_mut().zip(mapped[off..off + w * 4].chunks_exact(4)) {
                *dst = f32::from_le_bytes([src[0], src[1], src[2], src[3]]);
            }
        }
    }
    buffer.unmap();
    Ok(())
}

/// Stage one f32 field into little-endian scratch bytes, returning the
/// top-half mean, bottom-half mean and minimum in the same pass. The
/// caller reuses one scratch buffer across both uploads and the neutral
/// clears, so steady-state frames allocate nothing here.
pub(super) fn stage_field_bytes(
    scratch: &mut Vec<u8>,
    field: &[f32],
    width: usize,
    height: usize,
) -> (f32, f32, f32) {
    let n = field.len();
    if scratch.len() != n * 4 {
        scratch.resize(n * 4, 0);
    }
    let split = height.div_ceil(2).max(1);
    let stride = width.max(1);
    let mut top = 0.0f64;
    let mut bottom = 0.0f64;
    let mut top_n = 0u64;
    let mut bottom_n = 0u64;
    let mut min = f32::INFINITY;
    for (i, v) in field.iter().enumerate() {
        scratch[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        if i / stride < split {
            top += *v as f64;
            top_n += 1;
        } else {
            bottom += *v as f64;
            bottom_n += 1;
        }
        if *v < min {
            min = *v;
        }
    }
    let mean = |sum: f64, count: u64| {
        if count == 0 {
            0.0
        } else {
            (sum / count as f64) as f32
        }
    };
    (mean(top, top_n), mean(bottom, bottom_n), min)
}

/// Forward vertex stride in bytes: pos(12) + normal(12) + uv(8) + tangent(16).
/// Matches `MeshData::vertex_bytes` and `VIn` in `shaders/forward.wgsl`.
/// Units bytes per vertex. Determinism: same mesh bytes decode at these offsets.
/// Caller: `forward_pipeline` below builds `ra1-forward-pipeline` from these.
pub(super) const FORWARD_VERTEX_STRIDE: u64 = 48;
/// Byte offset of the tangent (location 3) in the forward vertex.
/// Units bytes from vertex start. Determinism: fixed layout, never hashed.
/// Caller: `forward_pipeline` below.
pub(super) const FORWARD_TANGENT_OFFSET: u64 = 32;
/// LF5 previous-position stride in bytes: one compact `vec3` (location 6),
/// a stream apart from the lighting vertex. Units bytes per vertex.
/// Determinism fixed layout, never hashed.
/// Caller: `forward_pipeline` below plus `Presenter::pack_mesh`.
pub(super) const PREV_POSITION_STRIDE: u64 = 12;
/// Vertex attribute of the LF5 previous-position stream: `VPrev` location 6.
const PREV_POSITION_ATTRIBUTES: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
    format: wgpu::VertexFormat::Float32x3,
    offset: 0,
    shader_location: 6,
}];
/// Group 0 binding of the tangent-space normal map sampled by `fs`/`fs_skinned`.
/// Matches `shaders/forward.wgsl` `normal_map` at 17.
/// Units binding index. Determinism: fixed layout, never hashed.
/// Caller: `forward_layout_tests` pin it to the shader; the presenter's
/// `forward_bind_with_sky` binds the view at this slot.
#[cfg(test)]
pub(super) const FORWARD_NORMAL_BINDING: u32 = 17;
/// Group 0 binding of the normal-map sampler at 18.
/// Matches `shaders/forward.wgsl` `normal_smp`.
/// Units binding index. Determinism: fixed layout, never hashed.
/// Caller: `forward_layout_tests` pin it to the shader; the presenter's
/// `forward_bind_with_sky` binds the sampler at this slot.
#[cfg(test)]
pub(super) const FORWARD_NORMAL_SAMPLER_BINDING: u32 = 18;

/// Fragment color targets for `formats` in attachment order: the HDR
/// target blends `REPLACE`; the integer id target takes no blend (an
/// integer format rejects one). Units formats in, states out.
/// Determinism fixed order, never hashed. Allocates once per pipeline
/// build, a cold path only.
/// Caller: `forward_pipeline` plus `sky_pipeline` below.
fn color_target_states(formats: &[wgpu::TextureFormat]) -> Vec<Option<wgpu::ColorTargetState>> {
    formats
        .iter()
        .map(|format| {
            // Integer targets are not blendable; the id channel and the A3
            // debug ids are plain uints, so they take no blend state.
            let blend = if *format == super::frame::ID_TARGET_FORMAT
                || *format == wgpu::TextureFormat::R16Uint
            {
                None
            } else {
                Some(wgpu::BlendState::REPLACE)
            };
            Some(wgpu::ColorTargetState {
                format: *format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })
        })
        .collect()
}

/// Vertex attributes of the static mesh entry `vs` (`VIn`): pos, normal, uv,
/// tangent. Offsets follow `MeshData::vertex_bytes`.
const STATIC_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 4] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 12,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 24,
        shader_location: 2,
    },
    // Location 3 carries the tangent with handedness in w.
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: FORWARD_TANGENT_OFFSET,
        shader_location: 3,
    },
];

/// Vertex attributes of the skinned entry `vs_skinned` (`SIn`): pos, normal,
/// uv, the tangent with handedness, four joint indices and four weights.
/// Offsets follow the 80-byte skinned vertex.
const SKINNED_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 6] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 12,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 24,
        shader_location: 2,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32x4,
        offset: 48,
        shader_location: 3,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 64,
        shader_location: 4,
    },
    // Location 5 carries the tangent with handedness in w (EG12 slice c).
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: SKINNED_TANGENT_OFFSET,
        shader_location: 5,
    },
];

/// Compilation options carrying the look guard preset (QL5): the shader's
/// `override POTATO_GUARD` is set here, so one source builds the fp32 look and
/// the potato look. An empty slice keeps wgpu's default (the shader default).
fn guard_options<'a>(constants: &'a [(&'a str, f64)]) -> wgpu::PipelineCompilationOptions<'a> {
    wgpu::PipelineCompilationOptions {
        constants,
        ..wgpu::PipelineCompilationOptions::default()
    }
}

pub(super) fn forward_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    ao_layout: &wgpu::BindGroupLayout,
    formats: &[wgpu::TextureFormat],
    multisample: wgpu::MultisampleState,
    constants: &[(&str, f64)],
) -> wgpu::RenderPipeline {
    forward_pipeline_for(
        device,
        shader,
        layout,
        ao_layout,
        formats,
        multisample,
        constants,
        false,
        false,
    )
}

/// The skinned twin of [`forward_pipeline`]: same shader, layout, fragment
/// entry, targets and depth state, with the `vs_skinned` vertex entry over
/// the 80-byte skinned vertex. A draw reads its joint block through the
/// dynamic offset at binding 7.
pub(super) fn forward_skinned_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    ao_layout: &wgpu::BindGroupLayout,
    formats: &[wgpu::TextureFormat],
    multisample: wgpu::MultisampleState,
    constants: &[(&str, f64)],
) -> wgpu::RenderPipeline {
    forward_pipeline_for(
        device,
        shader,
        layout,
        ao_layout,
        formats,
        multisample,
        constants,
        true,
        false,
    )
}

/// `constants` carries the look guard preset (`quality::look_guard_constants`):
/// the same shader source compiles to the fp32 look or the potato one.
#[expect(
    clippy::too_many_arguments,
    reason = "9 inputs mirror the forward pass"
)]
pub(super) fn forward_pipeline_for(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    ao_layout: &wgpu::BindGroupLayout,
    formats: &[wgpu::TextureFormat],
    multisample: wgpu::MultisampleState,
    constants: &[(&str, f64)],
    skinned: bool,
    debug: bool,
) -> wgpu::RenderPipeline {
    use crate::pipeline_warmup::CountedPipelines as _;
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ra1-forward-layout"),
        // Group 1 carries the LF3 occlusion field (the frame's GTAO
        // output); group 0 stays the per-draw forward group.
        bind_group_layouts: &[Some(layout), Some(ao_layout)],
        immediate_size: 0,
    });
    let targets = color_target_states(formats);
    // LF5: the 1x forward carries the motion target as its third
    // attachment, so only that shape reads the previous position stream.
    // The MSAA twin keeps HDR alone and its old vertex entries. A3: the
    // debug G-buffer also has three targets but reads no motion, so it
    // keeps the plain entries.
    let motion = formats.len() > 2 && !debug;
    let (entry, stride, attributes): (&str, u64, &[wgpu::VertexAttribute]) = match (skinned, motion)
    {
        (false, true) => (
            "vs_motion",
            FORWARD_VERTEX_STRIDE,
            &STATIC_VERTEX_ATTRIBUTES,
        ),
        (true, true) => (
            "vs_skinned_motion",
            SKINNED_VERTEX_STRIDE,
            &SKINNED_VERTEX_ATTRIBUTES,
        ),
        (true, false) => (
            "vs_skinned",
            SKINNED_VERTEX_STRIDE,
            &SKINNED_VERTEX_ATTRIBUTES,
        ),
        // Stride matches `MeshData::vertex_bytes`: pos + normal + uv + tangent.
        (false, false) => ("vs", FORWARD_VERTEX_STRIDE, &STATIC_VERTEX_ATTRIBUTES),
    };
    let vertex_buffers = [
        Some(wgpu::VertexBufferLayout {
            array_stride: stride,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes,
        }),
        // LF5: the previous positions ride a compact vec3 stream of their
        // own, never a second copy of the lighting vertex.
        Some(wgpu::VertexBufferLayout {
            array_stride: PREV_POSITION_STRIDE,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &PREV_POSITION_ATTRIBUTES,
        }),
    ];
    let buffers = if motion && !skinned {
        &vertex_buffers[..2]
    } else {
        &vertex_buffers[..1]
    };
    device.create_render_pipeline_counted(&wgpu::RenderPipelineDescriptor {
        label: Some(if skinned {
            "ra1-forward-skinned-pipeline"
        } else {
            "ra1-forward-pipeline"
        }),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(entry),
            buffers,
            compilation_options: guard_options(constants),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(if debug { "fs_gbuffer" } else { "fs" }),
            targets: &targets,
            compilation_options: guard_options(constants),
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Back),
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample,
        multiview_mask: None,
        cache: None,
    })
}

/// The depth-only twins of the forward pipelines: the same vertex entries
/// (`vs_depth` for the static meshes, `vs_skinned` for the skinned ones) and
/// no fragment stage. LF3 landing 2: the depth prepass writes the frame's
/// depth before the forward pass, so the GTAO kernel can build the
/// occlusion field the forward pass needs for its ambient. The depth state
/// matches the forward pass (same compare, same cull), so the prepass
/// leaves the same depth the forward pass would have written.
pub(super) fn forward_depth_pipelines(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    constants: &[(&str, f64)],
) -> [wgpu::RenderPipeline; 2] {
    [
        depth_pipeline(device, shader, layout, constants, false),
        depth_pipeline(device, shader, layout, constants, true),
    ]
}

/// One depth-only pipeline: the vertex entry, the vertex layout and the
/// depth state of the forward twin, with no fragment stage.
fn depth_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    constants: &[(&str, f64)],
    skinned: bool,
) -> wgpu::RenderPipeline {
    use crate::pipeline_warmup::CountedPipelines as _;
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ra1-depth-layout"),
        // The depth pass shades nothing, so it carries the forward group
        // alone: the occlusion field belongs to the forward pass.
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    let (entry, stride, attributes): (&str, u64, &[wgpu::VertexAttribute]) = if skinned {
        (
            "vs_skinned",
            SKINNED_VERTEX_STRIDE,
            &SKINNED_VERTEX_ATTRIBUTES,
        )
    } else {
        ("vs_depth", FORWARD_VERTEX_STRIDE, &STATIC_VERTEX_ATTRIBUTES)
    };
    device.create_render_pipeline_counted(&wgpu::RenderPipelineDescriptor {
        label: Some(if skinned {
            "ra1-depth-skinned-pipeline"
        } else {
            "ra1-depth-pipeline"
        }),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(entry),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: stride,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes,
            })],
            compilation_options: guard_options(constants),
        },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Back),
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Sky pipeline: one fullscreen triangle at the far plane. Depth test
/// `LessEqual` against the cleared 1.0 passes only where no geometry drew,
/// and nothing is written, so it costs the uncovered pixels alone.
pub(super) fn sky_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    formats: &[wgpu::TextureFormat],
    multisample: wgpu::MultisampleState,
) -> wgpu::RenderPipeline {
    use crate::pipeline_warmup::CountedPipelines as _;
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ra1-sky-layout"),
        // The sky reads the forward group alone: the occlusion field belongs
        // to the forward pass, which binds it before the sky triangle.
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    let targets = color_target_states(formats);
    device.create_render_pipeline_counted(&wgpu::RenderPipelineDescriptor {
        label: Some("ra1-sky-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_sky"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_sky"),
            targets: &targets,
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample,
        multiview_mask: None,
        cache: None,
    })
}

pub(super) fn blit_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    use crate::pipeline_warmup::CountedPipelines as _;
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ra1-blit-layout"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline_counted(&wgpu::RenderPipelineDescriptor {
        label: Some("ra1-blit-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            // A `*Srgb` target encodes on write: leave the encode to it.
            entry_point: Some(if format.is_srgb() { "fs_hw_srgb" } else { "fs" }),
            targets: &[Some(wgpu::ColorTargetState {
                format,
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
    })
}

pub(super) fn map_readback(
    device: &wgpu::Device,
    waits: &mut FrameWaits,
    buffer: &wgpu::Buffer,
    bytes_per_row: u32,
    padded: u32,
    height: u32,
) -> Result<Vec<u8>, PresentError> {
    waits.map_and_wait();
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| PresentError::Readback(format!("poll: {e}")))?;
    rx.recv()
        .map_err(|e| PresentError::Readback(format!("map channel: {e}")))?
        .map_err(|e| PresentError::Readback(format!("map: {e}")))?;
    let mut rgba = vec![0u8; (bytes_per_row as usize) * (height as usize)];
    {
        let mapped = slice
            .get_mapped_range()
            .map_err(|e| PresentError::Readback(format!("mapped range: {e}")))?;
        for (dst, src) in rgba.chunks_exact_mut(bytes_per_row as usize).enumerate() {
            let off = dst * padded as usize;
            src.copy_from_slice(&mapped[off..off + bytes_per_row as usize]);
        }
    }
    buffer.unmap();
    Ok(rgba)
}

/// WIN1 helper: build a `'static` surface from an owned window. The
/// surface keeps the window alive.
#[cfg(feature = "window")]
pub fn create_surface(
    instance: &wgpu::Instance,
    window: Arc<winit::window::Window>,
) -> Result<wgpu::Surface<'static>, PresentError> {
    instance
        .create_surface(window)
        .map_err(|e| PresentError::Device(format!("surface: {e}")))
}

#[cfg(test)]
mod cq7_present_alloc_tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    fn present_item(mesh: u64, tex: Option<u64>) -> DrawItem {
        DrawItem {
            mesh: MeshKey::new(mesh, [0x11; 32]),
            material: crate::v1::PbrMaterial {
                base_color: [0.5, 0.5, 0.5],
                metallic: 0.0,
                roughness: 0.5,
                exposure: 1.0,
            },
            base_color: tex.map(|t| TextureKey::new(t, [0x77; 32])),
            normal_map: None,
            orm: None,
            emissive: None,
        }
    }

    #[test]
    fn steady_120_frames_read_zero_allocs() {
        // Caller: `stage_frame` above (capture twin) plus the live order/camera/uniform slice.
        // Behavior: 120 steady frames of the submit CPU slice must not allocate.
        let scene = crate::v1::default_scene(160, 120).expect("scene valid");
        let mut items = Vec::with_capacity(48);
        for id in 0..48u64 {
            items.push(present_item(id, Some(id % 8)));
        }
        let list = DrawList {
            camera: scene.camera.clone(),
            light: scene.light.clone(),
            items,
        };
        let mut scratch = vec![0; 256];
        let warmed_cap = scratch.capacity();
        // Every draw's bound covers the scene target, so the cull keeps the
        // whole list and the measured slice is the same 120 frames.
        let target = scene.camera.target;
        let bounds = crate::present::gpu_present::submit_cull::DrawBounds {
            spheres: [[target.x, target.y, target.z, 2.0]; MAX_DRAWS_PER_FRAME],
            cullable: [true; MAX_DRAWS_PER_FRAME],
        };
        let ((), count) = measure(|| {
            for _ in 0..120u32 {
                let frame = effective_camera(&list, 160, 120).expect("camera builds");
                let (_, submitted) = live_submit_order(&list, &bounds, frame.built_view_proj);
                assert_eq!(submitted, list.items.len());
                let (cam_u, light_u, _) = crate::v1::build_uniforms_with_view_proj(
                    &frame,
                    frame.built_view_proj,
                    &list.light,
                    &scene.material,
                )
                .expect("uniforms build");
                // Present uniform image via stack arrays (mirrors fixed `stage_frame`).
                let cam_bytes = cam_u.to_array();
                let light_bytes = light_u.to_array();
                assert_eq!(cam_bytes.len(), 80);
                assert_eq!(light_bytes.len(), 32);
                for item in &list.items {
                    let mat = crate::v1::build_material(&item.material).expect("material builds");
                    let bytes = mat.to_array();
                    assert_eq!(bytes.len(), 32);
                }
                let field = [0.5f32; 64];
                let _ = stage_field_bytes(&mut scratch, &field, 8, 8);
                assert_eq!(scratch.capacity(), warmed_cap);
            }
        });
        assert_eq!(
            count.calls, 0,
            "steady present-submit slice must not allocate"
        );
    }
}

#[cfg(test)]
mod rl32_tests {
    use super::*;

    fn rl32_item(mesh: u64, tex: Option<u64>, rough: f32) -> DrawItem {
        DrawItem {
            mesh: MeshKey::new(mesh, [0x11; 32]),
            material: crate::v1::PbrMaterial {
                base_color: [0.5, 0.5, 0.5],
                metallic: 0.0,
                roughness: rough,
                exposure: 1.0,
            },
            base_color: tex.map(|t| TextureKey::new(t, [0x77; 32])),
            normal_map: None,
            orm: None,
            emissive: None,
        }
    }

    #[test]
    fn rl32_order_is_stable_and_groups_pipeline_adjacent() {
        let scene = crate::v1::default_scene(160, 120).expect("scene valid");
        let list = DrawList {
            camera: scene.camera.clone(),
            light: scene.light.clone(),
            items: vec![
                rl32_item(3, Some(7), 0.5),
                rl32_item(1, None, 0.9),
                rl32_item(3, Some(7), 0.5),
                rl32_item(4, Some(9), 0.5),
            ],
        };
        let mut a = [0usize; MAX_DRAWS_PER_FRAME];
        let mut b = [0usize; MAX_DRAWS_PER_FRAME];
        let first = rl32_submit_order(&list, &mut a).to_vec();
        let second = rl32_submit_order(&list, &mut b).to_vec();
        assert_eq!(first, second, "same order twice");
        let i0 = first.iter().position(|&d| d == 0).expect("draw 0 submits");
        let i2 = first.iter().position(|&d| d == 2).expect("draw 2 submits");
        assert_eq!(i0.abs_diff(i2), 1, "same group adjacent: {first:?}");
    }

    #[test]
    fn rl32_duplicate_draws_form_one_group() {
        let scene = crate::v1::default_scene(160, 120).expect("scene valid");
        let list = DrawList {
            camera: scene.camera.clone(),
            light: scene.light.clone(),
            items: vec![
                rl32_item(3, Some(7), 0.5),
                rl32_item(3, Some(7), 0.5),
                rl32_item(4, Some(9), 0.5),
            ],
        };
        let mut buf = [0usize; MAX_DRAWS_PER_FRAME];
        let order = rl32_submit_order(&list, &mut buf).to_vec();
        assert_eq!(order.len(), 3);
        let mut groups = 1usize;
        for w in 1..order.len() {
            if rl32_group_key(&list.items[order[w]]) != rl32_group_key(&list.items[order[w - 1]]) {
                groups += 1;
            }
        }
        assert_eq!(groups, 2, "duplicates share one group: {order:?}");
    }
}

#[cfg(test)]
mod forward_layout_tests {
    use super::{
        FORWARD_NORMAL_BINDING, FORWARD_NORMAL_SAMPLER_BINDING, FORWARD_TANGENT_OFFSET,
        FORWARD_VERTEX_STRIDE,
    };

    #[test]
    fn forward_stride_matches_mesh_bytes() {
        let mesh = crate::v1::cube();
        let stride = usize::try_from(FORWARD_VERTEX_STRIDE).expect("stride fits address space");
        let bytes = mesh.vertex_bytes();
        assert_eq!(
            bytes.len(),
            mesh.positions.len() * stride,
            "pipeline stride matches interleaved mesh bytes"
        );
        let off =
            usize::try_from(FORWARD_TANGENT_OFFSET).expect("tangent offset fits address space");
        assert_eq!(off, 32, "tangent starts after pos(12) + normal(12) + uv(8)");
        for (i, want) in mesh.tangents.iter().enumerate() {
            let base = i * stride + off;
            for (k, lane) in want.iter().enumerate() {
                let got = f32::from_le_bytes(
                    bytes[base + k * 4..base + (k + 1) * 4]
                        .try_into()
                        .expect("tangent bytes in range"),
                );
                assert_eq!(got, *lane, "tangent lane {k} round-trips at vertex {i}");
            }
        }
        assert_ne!(
            stride, 32,
            "32-byte stride drops location 3 and panics pipeline create"
        );
    }

    #[test]
    fn forward_shader_expects_tangent_at_three() {
        assert!(
            crate::v1_gpu::FORWARD_PBR_WGSL.contains("@location(3) tangent: vec4<f32>"),
            "vertex shader declares tangent at location 3"
        );
    }

    #[test]
    fn forward_shader_needs_normal_at_seventeen() {
        assert_eq!(
            FORWARD_NORMAL_BINDING, 17,
            "submit normal texture rides at group 0 binding 17"
        );
        assert_eq!(
            FORWARD_NORMAL_SAMPLER_BINDING, 18,
            "submit normal sampler rides at group 0 binding 18"
        );
        let wgsl = crate::v1_gpu::FORWARD_PBR_WGSL;
        assert!(
            wgsl.contains("@group(0) @binding(17) var normal_map"),
            "fragment shader declares normal_map at 17"
        );
        assert!(
            wgsl.contains("@group(0) @binding(18) var normal_smp"),
            "fragment shader declares normal_smp at 18"
        );
        assert!(
            wgsl.contains("textureSample(normal_map, normal_smp"),
            "fs samples the bound normal map, so the layout must carry 17"
        );
        // r3-skinned-pipeline uses `fs_skinned` from the same file: it
        // samples the same 17/18 pair, so its layout must carry both or
        // pipeline create fails with binding 17 missing.
        let skinned = wgsl
            .split("fn fs_skinned")
            .nth(1)
            .expect("skinned fragment entry exists");
        assert!(
            skinned.contains("textureSample(normal_map, normal_smp"),
            "fs_skinned samples the bound normal map, so its layout needs 17"
        );
    }
}

#[cfg(test)]
mod forward_id_attachment_tests {
    #[test]
    fn forward_id_attachment_count_is_two() {
        assert_eq!(
            super::forward_color_target_count(),
            3,
            "forward pass carries HDR, the R32Uint id target and the motion target"
        );
        assert_eq!(
            super::forward_color_target_formats(),
            [
                crate::present::gpu_present::HDR_FORMAT,
                crate::present::gpu_present::frame::ID_TARGET_FORMAT,
                crate::present::gpu_present::MOTION_FORMAT
            ],
            "attachment order is HDR, id, then motion"
        );
        assert_eq!(
            crate::present::gpu_present::frame::ID_TARGET_FORMAT,
            wgpu::TextureFormat::R32Uint,
            "id target is R32Uint"
        );
        assert_eq!(
            crate::present::gpu_present::frame::ID_CLEAR,
            u32::MAX,
            "id clears to no-draw"
        );
    }
}

#[cfg(test)]
mod sun_live_tests;

#[cfg(test)]
mod blit_live_tests;

#[cfg(test)]
mod eg8_capture_sky_tests {
    use super::*;

    #[test]
    fn capture_sky_state_round_trips_every_arm() {
        // Caller: `capture_sky_state`/`capture_sky_from_state` above.
        // Behavior: all three arms encode and decode by name, and an arm the
        // code never writes stays the live one, so no window or probe frame
        // silently switches its sky off.
        assert_eq!(capture_sky_state(None), CAPTURE_SKY_LIVE);
        assert_eq!(capture_sky_state(Some(false)), CAPTURE_SKY_OFF);
        assert_eq!(capture_sky_state(Some(true)), CAPTURE_SKY_ON);
        assert_eq!(capture_sky_from_state(CAPTURE_SKY_LIVE), None);
        assert_eq!(capture_sky_from_state(CAPTURE_SKY_OFF), Some(false));
        assert_eq!(capture_sky_from_state(CAPTURE_SKY_ON), Some(true));
        assert_eq!(
            capture_sky_from_state(CAPTURE_SKY_ON.saturating_add(1)),
            None,
            "an unknown state stays the live arm"
        );
        // The capture arm states the row itself: on draws inside the pass,
        // off skips, and no decision keeps the frame's own plan.
        assert_eq!(
            resolve_sky_plan(Some(true), SkyPlan::Skip, false),
            SkyPlan::InPass,
            "sky row on draws"
        );
        assert_eq!(
            resolve_sky_plan(Some(false), SkyPlan::OwnPass, false),
            SkyPlan::Skip,
            "sky row off skips"
        );
        assert_eq!(
            resolve_sky_plan(None, SkyPlan::OwnPass, false),
            SkyPlan::OwnPass,
            "no decision keeps the live plan"
        );
        assert!(
            forward_depth_should_store(true, SkyPlan::Skip),
            "capture stores depth"
        );
    }

    #[test]
    fn capture_twin_sky_pair_differs_and_repeats() {
        // Caller: `render_offscreen_with_toggles` above; EG8 row, "render the
        // arena frame with each look pass on and off; live only if a masked
        // region differs above epsilon".
        // Behavior: the capture twin draws the sky while the sky row is on and
        // keeps the clear color while it is off, so the two PNGs of one list
        // differ (a pixel change, never the row name), while the off frame x2
        // is byte-identical. Before this landing the twin passed `false` to
        // `encode_forward` for every frame, so the pair was identical.
        let probe = Presenter::init_offscreen().map_err(|error| match error {
            crate::present::PresentError::NoAdapter => {
                crate::test_gpu::NoAdapter("no wgpu adapter on this machine".into())
            }
            other => panic!("presenter init failed (must fail loudly): {other}"),
        });
        let Some(mut presenter) =
            crate::test_gpu::require_or_skip("capture_twin_sky_pair_differs_and_repeats", probe)
        else {
            return;
        };
        let key = MeshKey::new(3, [0xE8; 32]);
        presenter
            .upload_mesh(key, &v1::cube())
            .expect("cube uploads");
        presenter.resize(160, 120).expect("valid size applies");
        let scene = v1::default_scene(160, 120).expect("default scene is valid");
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
        let mut off = crate::pass_truth::LookToggles::all_on();
        off.disable("sky");
        let on_frame = presenter
            .render_offscreen_with_toggles(&list, &crate::pass_truth::LookToggles::all_on())
            .expect("sky-on frame renders");
        let off_frame = presenter
            .render_offscreen_with_toggles(&list, &off)
            .expect("sky-off frame renders");
        let off_again = presenter
            .render_offscreen_with_toggles(&list, &off)
            .expect("sky-off frame repeats");
        assert!(on_frame.receipt.is_clean(), "sky-on frame is clean");
        assert!(off_frame.receipt.is_clean(), "sky-off frame is clean");
        let on_png = on_frame.png.expect("sky-on frame has bytes");
        let off_png = off_frame.png.expect("sky-off frame has bytes");
        assert_eq!(
            off_png,
            off_again.png.expect("sky-off frame has bytes"),
            "the sky-off frame repeats byte-exact"
        );
        assert_ne!(
            on_png, off_png,
            "the sky pass moves pixels between the on and off capture frames"
        );
    }
}

#[cfg(test)]
mod forward_sky_plan_tests {
    use super::{forward_depth_should_store, resolve_sky_plan, SkyPlan};

    #[test]
    fn an_own_pass_sky_request_folds_into_the_pass_only_at_4x() {
        // Caller: `encode_forward` above. Behavior: with no capture decision
        // the live request stands on a 1x frame, so the sky is timed on its
        // own; a 4x frame cannot split its resolve, so it draws in the pass;
        // a capture decision always wins and always draws inside the pass.
        assert_eq!(
            resolve_sky_plan(None, SkyPlan::OwnPass, false),
            SkyPlan::OwnPass
        );
        assert_eq!(
            resolve_sky_plan(None, SkyPlan::OwnPass, true),
            SkyPlan::InPass
        );
        assert_eq!(resolve_sky_plan(None, SkyPlan::Skip, true), SkyPlan::Skip);
        assert_eq!(resolve_sky_plan(None, SkyPlan::Skip, false), SkyPlan::Skip);
        assert_eq!(
            resolve_sky_plan(Some(true), SkyPlan::Skip, false),
            SkyPlan::InPass
        );
        assert_eq!(
            resolve_sky_plan(Some(false), SkyPlan::OwnPass, true),
            SkyPlan::Skip
        );
    }

    #[test]
    fn depth_is_stored_only_while_something_reads_it() {
        // Caller: `encode_forward` above. Behavior: the own-pass sky and the
        // fog feed copy read depth after the pass; with no reader the pass
        // discards it. HDR pixels stay identical either way.
        assert!(!forward_depth_should_store(false, SkyPlan::Skip));
        assert!(!forward_depth_should_store(false, SkyPlan::InPass));
        assert!(forward_depth_should_store(false, SkyPlan::OwnPass));
        assert!(forward_depth_should_store(true, SkyPlan::Skip));
        assert!(forward_depth_should_store(true, SkyPlan::OwnPass));
    }

    #[test]
    fn the_live_frame_asks_for_the_sky_not_a_fixed_skip_switch() {
        // Caller: `present_frame_live` in live.rs. Behavior: the live encode
        // plans the sky from the frame's own toggle (no fixed switch hides
        // its cost), and the pass plan is resolved before the sky draws.
        let submit = include_str!("submit.rs");
        let live = include_str!("live.rs");
        let removed = concat!("FORWARD_SKY", "_SKIP");
        assert!(!submit.contains(removed), "the fixed skip switch is gone");
        let at = live
            .find("fn present_frame_live")
            .expect("live frame exists");
        let end = live[at..]
            .find("fn live_sky_row_us")
            .map_or(live.len(), |i| at + i);
        let body = &live[at..end];
        assert!(
            body.contains("ForwardLook::live(&look, sky_draw"),
            "live asks for the sky from its toggle"
        );
        assert!(
            submit.contains("sky: if sky { SkyPlan::OwnPass } else { SkyPlan::Skip }"),
            "an on toggle is the own pass"
        );
        let fwd = submit.find("fn encode_forward").expect("encode exists");
        let body = &submit[fwd..];
        let plan = body.find("resolve_sky_plan").expect("plan is resolved");
        let draw = body.find("rp.draw(0..3, 0..1)").expect("sky draw exists");
        assert!(plan < draw, "the plan is resolved before the sky draws");
    }
}
