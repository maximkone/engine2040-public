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
use super::submit::{effective_camera, inverse_view_proj_cols, map_readback_f32, padded_depth_row};

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
        let camera = effective_camera(list, width, height)?;
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
            sky_submit_us: Self::ms_to_us(look.sky_ms),
            layer_submit_us: Self::ms_to_us(look.layer_ms),
            particle_submit_us: Self::ms_to_us(look.particle_ms),
            decal_submit_us: Self::ms_to_us(look.decal_ms),
            ibl_submit_us: ibl.2,
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
        let (cam_bytes, light_bytes) = self.stage_frame(list, width, height)?;
        self.frame_waits = FrameWaits::default();
        self.ensure_targets(width, height);
        self.write_frame_uniforms(&cam_bytes, &light_bytes);
        self.bind_twin_shadow(Self::twin_focus_depth(list))?;
        self.write_fog_forward(list, width, height, true)?;
        if self.neutral_fog_pending {
            // Leaving the probe composite: the pair still holds its last
            // fog and occlusion, reset it once.
            let _rows = self.clear_fog_targets(width, height);
            self.neutral_fog_pending = false;
        }
        self.resolve_draw_binds(list)?;
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
            self.encode_blit(
                &mut enc,
                &targets.blit_bind,
                blit_view,
                blit_pipeline,
                timer.map(|(timer, _)| timer.blit_writes()),
            );
            if let Some((timer, slot)) = timer {
                timer.encode_resolve(&mut enc, slot);
            }
        }
        self.queue.submit(Some(enc.finish()));
        if let (Some(timer), Some(slot)) = (self.gpu_timer.as_ref(), timer_slot) {
            timer.arm(slot);
        }
        self.pop_scope(scope)?;
        self.finish_frame(FrameKind::Live);
        Ok(self.frame_receipt(
            list.items.len(),
            &LookPassReceipt::skipped(),
            (0, 0, 0),
            (0, 0, 0, 0, 0, 0),
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
        let (cam_bytes, light_bytes) = self.stage_frame(list, width, height)?;
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
            let camera = effective_camera(list, width, height)?;
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
