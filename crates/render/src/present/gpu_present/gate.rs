//! gate: the pass-truth gate's seams on the window's live frame.
//!
//! Continues `impl Presenter` from `live`. Owner: render lane (EG8), consumer
//! `pass_truth::harness` (the gate that renders each look pass on and off) and
//! `present_frame_live` (the switches it flips). Units: pixels for captures,
//! microseconds for the sky row. The switches are pure; the capture blocks on
//! the GPU by design and never runs in a window run.
use super::super::*;
use super::frame::LivePixels;
use super::live::{live_half_res_enabled, live_present_size, live_sky_should_count};
use super::presenter::Presenter;
use super::submit::map_readback;
use super::ROW_ALIGN;

/// Byte offset of the IBL cut word inside the forward SH uniform: the pad
/// word of coefficient 0 (`ibl_sh[0].w`, read by `ibl_gain` in the forward
/// shader). The CPU packer leaves it at 0.0.
pub(super) const IBL_CUT_OFFSET: u64 = 12;

/// The IBL cut word for one frame: 0.0 keeps the baked ambient, 1.0 cuts it.
/// Pure. Caller: `write_frame_uniforms` on every window-path frame.
#[must_use]
pub(super) fn ibl_cut_word(ibl_on: bool) -> f32 {
    if ibl_on {
        0.0
    } else {
        1.0
    }
}

/// What one look-toggle set switches in the window's live frame besides the
/// sky (`live_sky_should_draw` owns that row), one field per look pass the
/// live encode can really remove. A pass with no field (decals, particles,
/// material layers) has no draw in the live frame, so switching it changes
/// nothing and the gate reads it PROBE. Pure.
/// Caller: `present_frame_live`; the pass-truth harness drives it through
/// `render_live_offscreen_with_toggles`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LiveLook {
    /// Forward pass draws the list (`graph`); off leaves the cleared frame.
    pub(super) forward: bool,
    /// Sun depth map is recorded before the forward draws (`shadow_cluster`).
    pub(super) sun_depth: bool,
    /// Baked ambient (SH diffuse plus prefiltered specular) lights the surfaces (`ibl`).
    pub(super) ibl: bool,
    /// The blit's bloom halo runs (`post`).
    pub(super) bloom: bool,
    /// GTAO runs on the stored depth and lights `ao_tex` (`gtao`).
    pub(super) gtao: bool,
    /// The spot shadow atlas is recorded into the frame's own command buffer
    /// before the forward draws (`spot`).
    pub(super) spot: bool,
}

impl LiveLook {
    /// Read the live switches off a toggle set; a pass without a field
    /// leaves the frame as it is.
    #[must_use]
    pub(super) fn from_toggles(toggles: &crate::pass_truth::LookToggles) -> Self {
        Self {
            forward: toggles.is_on("graph"),
            sun_depth: toggles.is_on("shadow_cluster"),
            ibl: toggles.is_on("ibl"),
            bloom: toggles.is_on("post"),
            gtao: toggles.is_on("gtao"),
            spot: toggles.is_on("spot"),
        }
    }
}

impl Presenter {
    /// Render one draw list through the window's live frame path under a
    /// look-toggle set, then read the pixels back: the LDR twin plus the draw
    /// id of every pixel. The id target is refreshed to its clear first, so
    /// the ids describe this frame alone (uncovered pixels read
    /// `NO_DRAW_ID`). This is the pass-truth gate's capture: it blocks on the
    /// GPU by design and tallies as a capture frame, never as a live one.
    /// Units pixels in and out. Presentation only, never hashed.
    /// Caller: `pass_truth::harness` (EG8 live-frame arm).
    pub fn capture_live_pixels(
        &mut self,
        list: &DrawList,
        toggles: &crate::pass_truth::LookToggles,
    ) -> Result<LivePixels, PresentError> {
        self.check_device()?;
        if self.suspended() {
            return Err(PresentError::Validation(
                "pixel capture while suspended".into(),
            ));
        }
        let (width, height) = self.swap.size();
        let (render_w, render_h) = live_present_size(width, height, live_half_res_enabled());
        self.ensure_targets(render_w, render_h);
        self.clear_id_target()?;
        let receipt = self.render_live_offscreen_with_toggles(list, toggles)?;
        let targets = self
            .targets
            .as_ref()
            .ok_or_else(|| PresentError::Validation("pixel capture has no targets".into()))?;
        let (w, h) = (targets.width, targets.height);
        let bytes_per_row = w * 4;
        let padded = bytes_per_row.div_ceil(ROW_ALIGN) * ROW_ALIGN;
        let readback = |label| {
            self.creates.buffer();
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: u64::from(padded) * u64::from(h),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        };
        let rgba_buf = readback("pass-truth-rgba");
        let id_buf = readback("pass-truth-ids");
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pass-truth-readback"),
            });
        for (texture, buffer) in [
            (&targets.out_texture, &rgba_buf),
            (&targets.id_texture, &id_buf),
        ] {
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer,
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
        }
        self.queue.submit(Some(enc.finish()));
        self.frame_waits = FrameWaits::default();
        let rgba = map_readback(
            &self.device,
            &mut self.frame_waits,
            &rgba_buf,
            bytes_per_row,
            padded,
            h,
        )?;
        let id_bytes = map_readback(
            &self.device,
            &mut self.frame_waits,
            &id_buf,
            bytes_per_row,
            padded,
            h,
        )?;
        self.finish_frame(FrameKind::Capture);
        let ids = id_bytes
            .chunks_exact(4)
            .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
            .collect();
        Ok(LivePixels {
            width: w,
            height: h,
            rgba,
            ids,
            receipt,
        })
    }

    /// Refresh the id target to its clear word, so a frame's ids describe
    /// that frame alone (the live pass loads the target, it never clears it).
    /// Cold path only. Caller: `capture_live_pixels` above.
    fn clear_id_target(&self) -> Result<(), PresentError> {
        let targets = self
            .targets
            .as_ref()
            .ok_or_else(|| PresentError::Validation("id clear has no targets".into()))?;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &targets.id_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &targets.id_clear,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(targets.width * 4),
                rows_per_image: Some(targets.height),
            },
            wgpu::Extent3d {
                width: targets.width,
                height: targets.height,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    /// The `sky` cell of a live frame's per-pass row: the device time of the
    /// newest resolved own-pass sky (a few frames late, never waited for),
    /// counted only when the bake moves pixels off the fog ramp. 0 without
    /// timestamp queries or before the first sample lands.
    /// Caller: `present_frame_live` above.
    pub(super) fn live_sky_row_us(&self) -> u64 {
        let ms = self
            .gpu_timer
            .as_ref()
            .map_or(0.0, |timer| f64::from(timer.latest_sky_ms()));
        let gap = crate::sky::sky_live_on_off_gap(&self.sky_look).unwrap_or(0.0);
        if live_sky_should_count(ms, gap) {
            Self::ms_to_us(ms)
        } else {
            0
        }
    }
}

#[cfg(test)]
mod live_look_tests {
    use super::{ibl_cut_word, LiveLook, IBL_CUT_OFFSET};
    use crate::pass_truth::LookToggles;

    #[test]
    fn each_look_row_flips_only_its_own_live_switch() {
        // Caller: `present_frame_live`. Behavior: every row the live encode can
        // remove flips exactly its own switch; a row the live frame has no draw
        // for, and a name nobody owns, flip nothing.
        let all = LiveLook::from_toggles(&LookToggles::all_on());
        assert_eq!(
            all,
            LiveLook {
                forward: true,
                sun_depth: true,
                ibl: true,
                bloom: true,
                gtao: true,
                spot: true
            }
        );
        let flip = |row: &str| {
            let mut toggles = LookToggles::all_on();
            toggles.disable(row);
            LiveLook::from_toggles(&toggles)
        };
        assert_eq!(
            flip("graph"),
            LiveLook {
                forward: false,
                ..all
            }
        );
        assert_eq!(
            flip("shadow_cluster"),
            LiveLook {
                sun_depth: false,
                ..all
            }
        );
        assert_eq!(flip("ibl"), LiveLook { ibl: false, ..all });
        assert_eq!(
            flip("post"),
            LiveLook {
                bloom: false,
                ..all
            }
        );
        assert_eq!(flip("gtao"), LiveLook { gtao: false, ..all });
        assert_eq!(flip("spot"), LiveLook { spot: false, ..all });
        for row in [
            "sky",
            "material_layer",
            "particles_gpu",
            "decals",
            "no-such-pass",
        ] {
            assert_eq!(flip(row), all, "{row} owns no field here");
        }
    }

    #[test]
    fn the_ibl_cut_word_sits_in_the_pad_the_packer_leaves_at_zero() {
        // Caller: `write_frame_uniforms` and `ibl_gain` in the forward shader.
        // Behavior: a lit frame writes 0.0 and a cut frame 1.0 into the pad
        // word of coefficient 0, which is where the CPU packer already leaves
        // 0.0 (so a frame that never writes it stays lit).
        assert_eq!(ibl_cut_word(true), 0.0);
        assert_eq!(ibl_cut_word(false), 1.0);
        let mut packed = [1u8; 144];
        let mut sh = crate::ibl::IrradianceSh::zero();
        sh.coeffs[0] = [0.5, 0.25, 0.125];
        crate::ibl::sh_uniform_bytes_into(&sh, &mut packed);
        let at = usize::try_from(IBL_CUT_OFFSET).expect("offset fits");
        assert_eq!(&packed[at..at + 4], &0.0f32.to_le_bytes(), "pad stays 0.0");
        let shader = include_str!("../../shaders/forward.wgsl");
        assert!(
            shader.contains("clamp(ibl_sh[0].w, 0.0, 1.0)"),
            "the shader reads the same word"
        );
    }

    #[test]
    fn every_window_path_frame_writes_the_cut_word() {
        // Caller: the live, probe and capture frames. Behavior: all three
        // stage their uniforms through `write_frame_uniforms`, which writes the
        // cut word, so one frame's cut can never leak into the next.
        let live = include_str!("live.rs");
        let submit = include_str!("submit.rs");
        let call = concat!("self.write_frame", "_uniforms(");
        assert_eq!(live.matches(call).count(), 2, "live and probe frames");
        assert_eq!(submit.matches(call).count(), 1, "capture frame");
        let at = live.find("fn write_frame_uniforms").expect("writer exists");
        let body = &live[at..at + 700];
        assert!(body.contains("IBL_CUT_OFFSET") && body.contains("ibl_cut_word(ibl_on)"));
    }
}
