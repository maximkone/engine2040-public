//! submit: frame encode/submit plus free GPU helpers (window build).
//!
//! Continues `impl Presenter` from `presenter`; helpers used across the
//! split are `pub(super)`, everything else keeps its exact shape.
use super::super::*;
use super::frame::{
    decal_err, ibl_err, layer_err, particle_err, sky_err, FrameOutput, FrameReceipt,
    LookPassReceipt, Targets,
};
use super::live::FogRows;
use super::presenter::{Presenter, StagedUniforms};
use super::{DEPTH_FORMAT, HDR_CLEAR, HDR_FORMAT, OUTPUT_FORMAT, ROW_ALIGN};
use crate::v1::{self, Manifest};
use crate::{material_layer, particles_gpu, sky};
use std::sync::Arc;

/// Last sun depth submit wall time in us (evidence for `arena per-pass`).
/// Relaxed atomic, presentation only, never hashed.
/// Owner submit, consumer `render_offscreen` receipt plus `sun_shadow` tests.
/// Caller: `encode_sun_depth` below stores every live frame.
static SUN_LAST_US: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Revision over forward inputs (mesh, material, base-color, count).
/// Delegates to `live::live_forward_revision` (no new hash owner).
/// Units u64 out. Determinism: same list gives same revision twice.
/// Caller: `sun_shadow::SunDepthMap::encode` plus `submit_live_sun_map`.
#[must_use]
pub(crate) fn present_frame_revision(list: &DrawList) -> u64 {
    super::live::live_forward_revision(list)
}

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

impl Presenter {
    /// Per-frame torch bytes beside the light bytes, zeroed for clears.
    /// Stack image, allocates nothing. Reuses the `forward_bind` owner.
    /// Caller: `stage_frame` below stages the same bytes into `torch_buf`;
    /// tests in `present/tests_gpu.rs` pin the byte layout.
    pub(crate) fn torch_bytes(
        &self,
        list: &DrawList,
    ) -> [u8; super::forward_bind::TORCH_BLOCK_BYTES] {
        let frame = super::forward_bind::TorchFrame::for_draws(!list.items.is_empty());
        let mut out = [0u8; super::forward_bind::TORCH_BLOCK_BYTES];
        frame.pack_into(&mut out);
        out
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
    pub(super) fn encode_look_passes(&mut self) -> Result<LookPassReceipt, PresentError> {
        let frame = self.look_frame();
        let cam = self.look_cam();
        // RL8: the RA1 skybox plus the RL2 ambient read the same bound
        // bake every frame the RL4 submit binds below (RL7 fog follows in
        // the fog tranche off the same bake). A corrupt bake fails the
        // frame loudly here instead of tinting it.
        let sky_sample = self.skybox_sample(frame.sun_dir)?;
        let ambient = self.ibl_ambient();
        if !sky_sample.iter().all(|c| c.is_finite()) || !ambient.iter().all(|c| c.is_finite()) {
            return Err(PresentError::Validation(
                "rl8 look bake is not finite".into(),
            ));
        }
        let sky = sky::submit_gpu_sky(
            &self.device,
            &self.queue,
            &frame,
            &self.sky_capture,
            &mut self.sky_sub,
        )
        .map_err(sky_err)?;
        let layers = material_layer::submit_chart_gpu(
            &self.device,
            &self.queue,
            &frame,
            &mut self.layer_sub,
        )
        .map_err(layer_err)?;
        self.particles.step(particles_gpu::TICK_DT);
        let particles = self
            .particles
            .submit_gpu(&self.device, &self.queue, &frame, &mut self.particle_sub)
            .map_err(particle_err)?;
        let decal_pool = &self.decal_pool;
        let decal_receipt = decal_pool
            .submit_gpu(&self.device, &self.queue, &frame, &cam, &mut self.decal_sub)
            .map_err(decal_err)?;
        // Each submitter above is one submit, one map and one poll wait.
        self.frame_waits.map_and_wait_n(4);
        Ok(LookPassReceipt {
            passes: 4,
            validation_errors: sky.validation_errors
                + layers.validation_errors
                + particles.validation_errors
                + decal_receipt.validation_errors,
            sky_ms: sky.submit_ms,
            layer_ms: layers.submit_ms,
            particle_ms: particles.submit_ms,
            decal_ms: decal_receipt.submit_ms,
        })
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

    /// Live sun depth pass is owned by the EG11 lane (`sun_shadow.rs`).
    /// The forward Location[3] fix must not block capture while that lane
    /// lands its map type, so this step records zero shadow time and lets
    /// the forward pass run. Caller: `render_offscreen` below.
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
            format: HDR_FORMAT,
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ra1-depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            // COPY_SRC feeds the RL9 per-frame depth readback.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
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
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
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
            format: super::frame::ID_TARGET_FORMAT,
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
        self.targets = Some(Targets {
            width,
            height,
            hdr_view,
            #[cfg(test)]
            hdr_texture: hdr,
            out_view: out_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            out_texture,
            depth_view: depth_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_texture,
            fog_texture,
            ao_texture,
            id_texture,
            id_view,
            id_clear,
            fog_col_buf,
            blit_bind,
            msaa,
        });
    }

    /// Pick each draw's forward bind group for the frame: its resident
    /// texture's group (built when the texture uploaded), or the 1x1 white
    /// fallback for `None` draws (identity multiply). A pure lookup into
    /// scratch the presenter keeps: no bind group is created here. Staged
    /// lists already proved every key resident, so a missing group fails
    /// with `InvalidDraw`, never a caller panic; callers run this after
    /// `stage_frame`.
    pub(super) fn resolve_draw_binds(&mut self, list: &DrawList) -> Result<(), PresentError> {
        self.draw_binds.clear();
        for (index, item) in list.items.iter().enumerate() {
            let choice = match &item.base_color {
                None => None,
                Some(key) => {
                    let found = self
                        .texture_binds
                        .iter()
                        .position(|(known, _)| known == key);
                    if found.is_none() {
                        return Err(PresentError::InvalidDraw {
                            index,
                            reason: "unknown texture key".into(),
                        });
                    }
                    found
                }
            };
            self.draw_binds.push(choice);
        }
        Ok(())
    }

    /// Bind group chosen for draw `draw` by `resolve_draw_binds`.
    fn draw_bind(&self, draw: usize) -> Option<&wgpu::BindGroup> {
        match self.draw_binds.get(draw)? {
            Some(index) => self.texture_binds.get(*index).map(|(_, bind)| bind),
            None => Some(&self.fallback_bind),
        }
    }

    /// Stage one frame: validate the list, camera and light, resolve every
    /// mesh key, and pack the uniform byte images. Returns camera and light
    /// bytes; the per-draw materials land in `material_scratch`, one
    /// 256-byte stride each, ready for the ring write. Fails before any GPU
    /// work is encoded.
    pub(super) fn stage_frame(
        &mut self,
        list: &DrawList,
        width: u32,
        height: u32,
    ) -> Result<StagedUniforms, PresentError> {
        list.validate()?;
        let camera = effective_camera(list, width, height)?;
        let aspect = camera.aspect;
        list.light
            .validate()
            .map_err(|e| PresentError::InvalidDraw {
                index: 0,
                reason: format!("light: {e}"),
            })?;
        // Camera plus light bytes come from a fixed valid material: the
        // packer ignores it for those uniforms, and every real material
        // already passed `DrawList::validate` above.
        let (cam_u, light_u, _) = v1::build_uniforms(
            &camera,
            aspect,
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
            reason: format!("frame: {e}"),
        })?;
        self.material_scratch.clear();
        for (index, item) in list.items.iter().enumerate() {
            let (_, _, mat_u) = v1::build_uniforms(&camera, aspect, &list.light, &item.material)
                .map_err(|e| PresentError::InvalidDraw {
                    index,
                    reason: format!("material: {e}"),
                })?;
            if self.meshes.get(&item.mesh).is_none() {
                return Err(PresentError::InvalidDraw {
                    index,
                    reason: format!("unknown mesh key {:?}", item.mesh.id),
                });
            }
            // Texture residency: unknown texture keys fail with their draw
            // index, mirroring meshes, before any GPU work is encoded.
            if let Some(key) = &item.base_color {
                if self.textures.get(key).is_none() {
                    return Err(PresentError::InvalidDraw {
                        index,
                        reason: "unknown texture key".into(),
                    });
                }
            }
            self.material_scratch.extend_from_slice(&mat_u.to_bytes());
            self.material_scratch
                .resize((index + 1) * MATERIAL_STRIDE, 0);
        }
        // Torch bytes beside the light bytes; zeroed for clears so clears
        // stay clean. Staged into `torch_buf` for the forward bind.
        let torch = self.torch_bytes(list);
        self.queue.write_buffer(&self.torch_buf, 0, &torch);
        Ok((cam_u.to_bytes(), light_u.to_bytes()))
    }

    /// Twin sun-shadow bind: resolve the frame focus depth into the
    /// twin sun band and refresh the twin shadow uniform the window
    /// mesh forward reads at group 0 binding 3 under
    /// `shaders/forward.wgsl` (the `fs` entry). One 32-byte uniform
    /// write per live frame; steady-state frames allocate nothing.
    /// A non-finite depth fails the frame loudly, never a silent band.
    pub(super) fn bind_twin_shadow(&self, view_depth_m: f32) -> Result<(), PresentError> {
        let fail = |e: crate::twin_shadow::TwinShadowError| PresentError::InvalidDraw {
            index: 0,
            reason: format!("rl19 twin shadow: {e}"),
        };
        let band =
            crate::twin_shadow::twin_active_cascade(&self.twin_sun, view_depth_m).map_err(fail)?;
        self.queue.write_buffer(
            &self.twin_shadow_buf,
            0,
            &crate::twin_shadow::twin_shadow_uniform(&self.twin_sun, band),
        );
        Ok(())
    }

    /// Frame focus depth in meters: eye-to-target distance, the cascade
    /// the frame subject lives in. Non-finite cameras fail loudly at the
    /// bind, never a silent band.
    pub(super) fn twin_focus_depth(list: &DrawList) -> f32 {
        (list.camera.eye - list.camera.target).length()
    }

    /// Forward pass: clear, draw the list, then (live frames) the sky at
    /// the far plane so the depth test rejects every covered pixel.
    /// `timestamps` brackets the pass on the device clock when supported.
    pub(super) fn encode_forward(
        &self,
        enc: &mut wgpu::CommandEncoder,
        targets: &Targets,
        list: &DrawList,
        sky: bool,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        // EG13 on-path choice: on plus built resolves 4x into 1x HDR, else
        // today's 1x path. Names the helper in production (not test-only).
        let (_, should_resolve) =
            super::msaa::msaa_forward_choice(&self.msaa, targets.msaa.is_some());
        let msaa_pair = targets.msaa.as_ref().filter(|_| should_resolve);
        let (color_view, depth_view, pipeline, resolve) = match msaa_pair {
            Some(pair) => (
                &pair.color_view,
                &pair.depth_view,
                &self.forward_hdr_msaa,
                Some(&targets.hdr_view),
            ),
            None => (
                &targets.hdr_view,
                &targets.depth_view,
                &self.forward_hdr,
                None,
            ),
        };
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ra1-forward"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: resolve,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(HDR_CLEAR),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    // Stored: the RL9 fog feed copies this frame's depth
                    // after the forward pass.
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: timestamps,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(pipeline);
        for (draw, item) in list.items.iter().enumerate() {
            let (Some(mesh), Some(bind)) = (self.meshes.get(&item.mesh), self.draw_bind(draw))
            else {
                continue;
            };
            // Material `draw` sits at one 256-byte stride in the ring.
            let offset = u32::try_from(draw * MATERIAL_STRIDE).unwrap_or(0);
            rp.set_bind_group(0, bind, &[offset]);
            rp.set_vertex_buffer(0, mesh.vertex.slice(..));
            rp.set_index_buffer(mesh.index.slice(..), wgpu::IndexFormat::Uint32);
            rp.draw_indexed(0..mesh.count, 0, 0..1);
        }
        if sky {
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
            rp.set_bind_group(0, &self.fallback_bind, &[0]);
            rp.draw(0..3, 0..1);
        }
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

    pub(super) fn pop_scope(&self, scope: wgpu::ErrorScopeGuard) -> Result<(), PresentError> {
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(PresentError::Validation(format!("{err:?}")));
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
    /// bytes plus manifest and receipt. While suspended (minimized) no GPU
    /// work runs and `png` is `None`; the last-good pipeline is kept.
    /// This is the explicit capture path: it records the four look passes
    /// (sky, layers, particles, decals) under one shared look bind, then
    /// the IBL chart plus the GTAO plus fog tranche (chart from the
    /// init-time bake, occlusion plus height fog plus contact from this
    /// frame's depth feed) whose pair the blit composites into the twin,
    /// and reads the pixels back. It blocks on the GPU by design, so its
    /// waits are tallied as capture frames, never as live frames.
    pub fn render_offscreen(&mut self, list: &DrawList) -> Result<FrameOutput, PresentError> {
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
        let (cam_bytes, light_bytes) = self.stage_frame(list, width, height)?;
        self.frame_waits = FrameWaits::default();
        let look = self.encode_look_passes()?;
        self.ensure_targets(width, height);
        self.write_frame_uniforms(&cam_bytes, &light_bytes);
        // The window twin forward path samples the landed cascaded
        // sun; the frame focus depth selects the active cascade.
        self.bind_twin_shadow(Self::twin_focus_depth(list))?;
        self.write_fog_forward(list, width, height, false)?;
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
        {
            let targets = self.targets.as_ref().expect("ensured above");
            let (_, _, depth_rb) = self.depth_rb.as_ref().expect("ensured above");
            self.encode_forward(&mut enc, targets, list, false, None);
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
        // contact passes on the device, upload the composite pair.
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
        let fog_rows: FogRows = if fog_wired {
            let camera = effective_camera(list, width, height)?;
            self.run_fog_tranche(&raw_depth, &camera, width, height)?
        } else {
            self.clear_fog_targets(width, height)
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
            self.encode_blit(
                &mut blit_enc,
                &targets.blit_bind,
                &targets.out_view,
                &self.twin_pipeline,
                None,
            );
            blit_enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &targets.out_texture,
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
            .map_err(|e| PresentError::Readback(format!("png: {e}")))?;
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
        let acquire_started = std::time::Instant::now();
        let current = self
            .surface
            .as_ref()
            .expect("checked above")
            .get_current_texture();
        self.acquire_us = self.acquire_us.saturating_add(
            u64::try_from(acquire_started.elapsed().as_micros()).unwrap_or(u64::MAX),
        );
        self.acquire_frames = self.acquire_frames.saturating_add(1);
        let (width, height) = self.swap.size();
        match current {
            wgpu::CurrentSurfaceTexture::Success(frame) => {
                self.present_frame(frame, list, width, height)
            }
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                let receipt = self.present_frame(frame, list, width, height)?;
                self.reconfigure_surface();
                Ok(receipt)
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

/// The shared buffers and sampler every forward bind group points at.
pub(super) struct ForwardParts<'a> {
    pub(super) layout: &'a wgpu::BindGroupLayout,
    pub(super) cam: &'a wgpu::Buffer,
    pub(super) light: &'a wgpu::Buffer,
    pub(super) material: &'a wgpu::Buffer,
    pub(super) shadow: &'a wgpu::Buffer,
    pub(super) sampler: &'a wgpu::Sampler,
    pub(super) fog: &'a wgpu::Buffer,
    pub(super) normal_view: &'a wgpu::TextureView,
}

/// One forward bind group over `parts` and the base-color `view`. The
/// material binding spans a single stride-sized window of the ring; the
/// draw supplies the dynamic offset, so one group serves every draw that
/// samples this texture.
pub(super) fn forward_bind_group(
    device: &wgpu::Device,
    parts: &ForwardParts<'_>,
    view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ra1-forward-bind"),
        layout: parts.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: parts.cam.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: parts.light.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: parts.material,
                    offset: 0,
                    size: wgpu::BufferSize::new(MATERIAL_BYTES),
                }),
            },
            // The twin shadow uniform the mesh forward reads at binding 3.
            wgpu::BindGroupEntry {
                binding: 3,
                resource: parts.shadow.as_entire_binding(),
            },
            // Base-color map plus sampler (bindings 4-5).
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(parts.sampler),
            },
            // Height fog uniform (binding 6).
            wgpu::BindGroupEntry {
                binding: 6,
                resource: parts.fog.as_entire_binding(),
            },
            // Tangent-space normal map plus its sampler (bindings 17-18).
            // The `fs`/`fs_skinned` entries sample them every fragment, so a
            // bind group without them leaves binding 17 missing in the
            // `ra1-forward-pipeline` layout and fails pipeline create.
            wgpu::BindGroupEntry {
                binding: FORWARD_NORMAL_BINDING,
                resource: wgpu::BindingResource::TextureView(parts.normal_view),
            },
            wgpu::BindGroupEntry {
                binding: FORWARD_NORMAL_SAMPLER_BINDING,
                resource: wgpu::BindingResource::Sampler(parts.sampler),
            },
        ],
    })
}

pub(super) fn uniform_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Effective frame camera: the draw-list camera with the aspect the
/// forward pass renders at. One writer: `stage_frame` and the RL9 fog
/// feed both go through here so the decode can never disagree with the
/// frame that produced the depth.
pub(super) fn effective_camera(
    list: &DrawList,
    width: u32,
    height: u32,
) -> Result<v1::Camera, PresentError> {
    let mut camera = list.camera.clone();
    camera.aspect = width as f32 / height as f32;
    camera.view_proj().map_err(|e| PresentError::InvalidDraw {
        index: 0,
        reason: format!("camera: {e}"),
    })?;
    Ok(camera)
}

/// Inverse view-projection in column-major `to_cols_array` order (the
/// same image the camera uniform carries) for the RL9 depth decode.
pub(super) fn inverse_view_proj_cols(camera: &v1::Camera) -> Result<[[f32; 4]; 4], PresentError> {
    let view_proj = camera.view_proj().map_err(|e| PresentError::InvalidDraw {
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
/// row padding. Mirrors `map_readback` for the color twin.
pub(super) fn map_readback_f32(
    device: &wgpu::Device,
    waits: &mut FrameWaits,
    buffer: &wgpu::Buffer,
    width: u32,
    padded: u32,
    height: u32,
) -> Result<Vec<f32>, PresentError> {
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
    let mut out = vec![0.0f32; w * height as usize];
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
    Ok(out)
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
/// Group 0 binding of the tangent-space normal map sampled by `fs`/`fs_skinned`.
/// Matches `shaders/forward.wgsl` `normal_map` at 17.
/// Units binding index. Determinism: fixed layout, never hashed.
/// Caller: `forward_bind_group` below binds this view for every draw.
pub(super) const FORWARD_NORMAL_BINDING: u32 = 17;
/// Group 0 binding of the normal-map sampler at 18.
/// Matches `shaders/forward.wgsl` `normal_smp`.
/// Units binding index. Determinism: fixed layout, never hashed.
/// Caller: `forward_bind_group` below binds the shared sampler here.
pub(super) const FORWARD_NORMAL_SAMPLER_BINDING: u32 = 18;

pub(super) fn forward_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    multisample: wgpu::MultisampleState,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ra1-forward-layout"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ra1-forward-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                // Stride matches `MeshData::vertex_bytes`: pos + normal + uv + tangent.
                // Location 3 carries the tangent with handedness in w.
                array_stride: FORWARD_VERTEX_STRIDE,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
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
                        format: wgpu::VertexFormat::Float32x4,
                        offset: FORWARD_TANGENT_OFFSET,
                        shader_location: 3,
                    },
                ],
            })],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
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

/// Sky pipeline: one fullscreen triangle at the far plane. Depth test
/// `LessEqual` against the cleared 1.0 passes only where no geometry drew,
/// and nothing is written, so it costs the uncovered pixels alone.
pub(super) fn sky_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    multisample: wgpu::MultisampleState,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ra1-sky-layout"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
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
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ra1-blit-layout"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
pub fn create_surface(
    instance: &wgpu::Instance,
    window: Arc<winit::window::Window>,
) -> Result<wgpu::Surface<'static>, PresentError> {
    instance
        .create_surface(window)
        .map_err(|e| PresentError::Device(format!("surface: {e}")))
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
    }
}
