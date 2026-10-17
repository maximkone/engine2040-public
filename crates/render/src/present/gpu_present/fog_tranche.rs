//! fog_tranche: per-frame fog plus GTAO passes and the neutral reset, on `Presenter`.
//!
//! Units: pixels for sizes, microseconds for the submit times in the receipt
//! tuple. The fields stay in `presenter`; this file only holds the methods
//! the live frame calls once per presented frame.
use super::super::*;
use super::frame::gtao_err;
use super::submit::{inverse_view_proj_cols, stage_field_bytes, FrameCamera};
use super::Presenter;

impl Presenter {
    /// Fog tranche plus GTAO: decode this frame's depth readback
    /// into the fog feed, run the height-fog plus contact passes on the
    /// device, run half-res GTAO plus its bilateral upsample over the
    /// same feed, and upload the composite pair plus the sRGB ramp color
    /// for the blit. Returns (fog passes, fog validation errors, fog
    /// submit us, gtao passes, gtao validation errors, gtao submit us)
    /// for the receipt and records the halve-means plus contact minimum
    /// as evidence.
    pub(super) fn run_fog_tranche(
        &mut self,
        raw_depth: &[f32],
        camera: &FrameCamera,
        width: u32,
        height: u32,
    ) -> Result<(u32, u32, u64, u32, u32, u64), PresentError> {
        let inv = inverse_view_proj_cols(camera)?;
        let feed =
            super::super::decode_depth_feed(raw_depth, width, height, inv, camera.eye.to_array())?;
        let frame = self.look_frame();
        let params = self.look_fog_params()?;
        // RL10: sun-scatter tint ahead of plan/submit; forward derived
        // from eye->target without a camera dependency (no heap, no
        // new crate). Tinted params flow into both below.
        let (dx, dy, dz) = (
            camera.target.x - camera.eye.x,
            camera.target.y - camera.eye.y,
            camera.target.z - camera.eye.z,
        );
        let len = (dx * dx + dy * dy + dz * dz).sqrt();
        let forward = if len > 0.0 {
            [dx / len, dy / len, dz / len]
        } else {
            [0.0, 0.0, -1.0]
        };
        let params = params
            .with_sun_tint(frame.sun_dir, forward)
            .map_err(|e| super::super::fog_err("tint", e))?;
        let contact = super::super::contact_for_frame(width, height)?;
        self.ensure_fog_plan(&params, &contact, width, height)?;
        let out = {
            let device = &self.device;
            let queue = &self.queue;
            let submitter = &mut self.fog_sub;
            submitter
                .submit(
                    device,
                    queue,
                    &feed.depth_m,
                    &feed.height_m,
                    &feed.normals,
                    &params,
                    &contact,
                    &frame,
                )
                .map_err(|e| super::super::fog_err("submit", e))?
        };
        self.frame_waits.map_and_wait();
        let w = width as usize;
        let h = height as usize;
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let layout = wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        };
        let (top, bottom, _) = stage_field_bytes(&mut self.fog_scratch, &out.fog.factor, w, h);
        {
            let targets = self.targets.as_ref().expect("fog targets ensured above");
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &targets.fog_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.fog_scratch,
                layout,
                size,
            );
        }
        let (ao_top, ao_bottom, ao_min) =
            stage_field_bytes(&mut self.fog_scratch, &out.contact.values, w, h);
        {
            let targets = self.targets.as_ref().expect("fog targets ensured above");
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &targets.ao_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.fog_scratch,
                layout,
                size,
            );
            // Linear radiance: the blit mixes it into the linear HDR frame
            // before the one tone map.
            let bytes = super::super::fog_blit_uniform_bytes(params.fog_color, width, height);
            self.queue.write_buffer(&targets.fog_col_buf, 0, &bytes);
        }
        self.fog_top_mean = top;
        self.fog_bottom_mean = bottom;
        self.fog_contact_min = ao_min;
        self.fog_ao_top_mean = ao_top;
        self.fog_ao_bottom_mean = ao_bottom;
        let us = (out.receipt.submit_ms * 1000.0) as u64;
        self.fog_submit_us = us;
        // Half-res GTAO plus its bilateral upsample over this
        // frame's decoded feed (the decode above guarantees positive
        // finite depth, so the submit validates exactly like fog's).
        let gtao_config = Self::gtao_config_for_frame(width, height)?;
        let gtao = {
            let device = &self.device;
            let queue = &self.queue;
            let submitter = &mut self.gtao_sub;
            submitter
                .submit(
                    device,
                    queue,
                    &feed.depth_m,
                    &feed.normals,
                    &gtao_config,
                    &frame,
                )
                .map_err(gtao_err)?
        };
        self.frame_waits.map_and_wait();
        let gus = Self::ms_to_us(gtao.submit_ms);
        self.gtao_submit_us = gus;
        // The pair now holds this frame's fog plus AO: the fog-off
        // branches below must reset it once before trusting neutral.
        self.fog_pair_dirty = true;
        // Live GTAO drives the blit AO: overwrite the contact pair above
        // with the full-res bilateral upsample over this frame's feed.
        // Same row-major layout as the contact upload; fog factor stays.
        {
            let full_len = self.gtao_sub.full_ao().len();
            if full_len != w.saturating_mul(h) {
                return Err(PresentError::Readback(format!(
                    "gtao full {full_len} texels mismatches frame {width}x{height}"
                )));
            }
            for (i, v) in self.gtao_sub.full_ao().iter().enumerate() {
                if !v.is_finite() {
                    return Err(PresentError::Readback(format!(
                        "gtao full_ao[{i}] is not finite"
                    )));
                }
            }
        }
        {
            let (scratch, ao) = (&mut self.fog_scratch, &self.gtao_sub);
            stage_field_bytes(scratch, ao.full_ao(), w, h);
        }
        {
            let targets = self
                .targets
                .as_ref()
                .ok_or_else(|| PresentError::Validation("live frame has no targets".into()))?;
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &targets.ao_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.fog_scratch,
                layout,
                size,
            );
        }
        Ok((
            2,
            out.receipt.validation_errors,
            us,
            2,
            gtao.validation_errors,
            gus,
        ))
    }

    /// RL9 neutral path: factor 0 (no tint), AO open, evidence reset.
    /// Held, fog-off and over-budget frames show the clear forward frame;
    /// GTAO rides the same gate (no feed, no occlusion pass). Leaves the
    /// pair neutral and clears `fog_pair_dirty`; callers skip this whole
    /// upload while the flag stays false.
    pub(crate) fn clear_fog_targets(
        &mut self,
        width: u32,
        height: u32,
    ) -> (u32, u32, u64, u32, u32, u64) {
        let n = width as usize * height as usize;
        if self.fog_scratch.len() != n * 4 {
            self.fog_scratch.resize(n * 4, 0);
        }
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let layout = wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        };
        let targets = self.targets.as_ref().expect("fog targets ensured above");
        self.fog_scratch.fill(0);
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &targets.fog_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.fog_scratch,
            layout,
            size,
        );
        let one = 1.0f32.to_le_bytes();
        for chunk in self.fog_scratch.chunks_exact_mut(4) {
            chunk.copy_from_slice(&one);
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &targets.ao_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.fog_scratch,
            layout,
            size,
        );
        // Always upload the frame size: the blit texel clamp needs a
        // valid range even when the factor stays 0 (a zero-size uniform
        // is an undefined clamp and blackened fresh presenters).
        let bytes = super::super::fog_blit_uniform_bytes([0.0, 0.0, 0.0], width, height);
        self.queue.write_buffer(&targets.fog_col_buf, 0, &bytes);
        self.fog_top_mean = 0.0;
        self.fog_bottom_mean = 0.0;
        self.fog_contact_min = 1.0;
        self.fog_ao_top_mean = 1.0;
        self.fog_ao_bottom_mean = 1.0;
        self.fog_submit_us = 0;
        self.gtao_submit_us = 0;
        self.fog_pair_dirty = false;
        (0, 0, 0, 0, 0, 0)
    }
}
