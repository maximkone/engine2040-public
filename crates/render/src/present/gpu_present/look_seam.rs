//! look_seam: where the E11 look table meets a frame.
//!
//! One place owns the frame's display plan. With no table bound the blit
//! writes the sink itself, exactly as it did before the look pass existed.
//! With a table bound the blit, the HUD and the debug lines land in the LDR
//! twin and [`Presenter::encode_look_pass`] records the table pass between
//! them and the display write: straight into the acquired surface view, or
//! into the twin's look target when the frame stays on the device.
//!
//! Units: pixels for sizes, a table digest for the bound look. Presentation
//! only, never hashed; a LUT-free frame records nothing here.

use super::super::*;
use super::live::LiveSink;
use super::presenter::Presenter;

mod upscale_gpu;
mod upscale_live;
pub(super) use upscale_gpu::SpanStamps;
pub(super) use upscale_live::UpscaleLive;

/// Where a window frame's pre-look pass writes and where its display write
/// reads.
#[derive(Clone, Copy)]
pub(super) struct LookStage<'a> {
    /// Where the blit, the HUD and the debug lines land.
    pub(super) blit: &'a wgpu::TextureView,
    /// Blit pipeline for [`Self::blit`]'s format.
    pub(super) pipeline: &'a wgpu::RenderPipeline,
    /// [`Self::blit`]'s target format (the HUD and line pipelines take it).
    pub(super) format: wgpu::TextureFormat,
    /// The look pass target plus its format; `None` while no table is bound.
    pub(super) display: Option<(&'a wgpu::TextureView, wgpu::TextureFormat)>,
}

impl Presenter {
    /// The display plan of one window-path frame. A missing surface pipeline
    /// or target set is a typed error, never a panic.
    pub(super) fn look_stage<'a>(
        &'a self,
        sink: LiveSink<'a>,
    ) -> Result<LookStage<'a>, PresentError> {
        let surface = match sink {
            LiveSink::Surface(view) => Some((
                view,
                self.surface_pipeline
                    .as_ref()
                    .ok_or(PresentError::NoSurface)?,
            )),
            LiveSink::Twin => None,
        };
        let targets = || {
            self.targets
                .as_ref()
                .ok_or_else(|| PresentError::Validation("window frame has no targets".into()))
        };
        if self.look_lut_bound() {
            let targets = targets()?;
            // The blit lands in the twin; the look pass writes the display
            // target, straight into the surface view or into the twin's own
            // look target when the frame stays on the device.
            let display = match surface {
                Some((view, (format, _))) => Some((view, *format)),
                None => Some((&targets.lut_out_view, OUTPUT_FORMAT)),
            };
            return Ok(LookStage {
                blit: &targets.out_view,
                pipeline: &self.twin_pipeline,
                format: OUTPUT_FORMAT,
                display,
            });
        }
        match surface {
            Some((view, (format, pipeline))) => Ok(LookStage {
                blit: view,
                pipeline,
                format: *format,
                display: None,
            }),
            None => {
                let targets = targets()?;
                Ok(LookStage {
                    blit: &targets.out_view,
                    pipeline: &self.twin_pipeline,
                    format: OUTPUT_FORMAT,
                    display: None,
                })
            }
        }
    }

    /// Record the look pass when `stage` carries a bound table: one fullscreen
    /// triangle from `src` (the graded frame) into the display target. No
    /// table means no pass, so the frame keeps the blit's own bytes.
    pub(super) fn encode_look_pass(
        &self,
        enc: &mut wgpu::CommandEncoder,
        stage: LookStage<'_>,
        src: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        let Some((display, format)) = stage.display else {
            return;
        };
        let Some(bind) = self
            .lut_pass
            .bind(&self.device, &self.lut, src, width, height)
        else {
            return;
        };
        let pipeline = self.lut_pass.pipeline_for(&self.device, format);
        self.lut_pass.encode(enc, &bind, &pipeline, display);
    }

    /// Push the chain's look to the GPU (E11): uploads `cfg.lut` when its
    /// digest changed, drops the texture when the look cleared, and does
    /// nothing when the bound table already matches. Call it at setup and on
    /// every look change, never per frame. Presentation only, never hashed.
    ///
    /// # Errors
    ///
    /// [`PresentError::InvalidTexture`] when the table cannot be uploaded; the
    /// previous table stays bound so the frame keeps its last good look.
    pub fn set_look(&mut self, cfg: &crate::post::ChainConfig) -> Result<(), PresentError> {
        self.lut
            .sync(&self.device, &self.queue, cfg.lut.as_ref())
            .map(|_| ())
            .map_err(|error| PresentError::InvalidTexture(error.to_string()))?;
        // Warm the look pass for both formats a frame can end in (the LDR
        // twin and the attached surface), so the first LUT frame encodes
        // instead of compiling.
        let _ = self.lut_pass.pipeline_for(&self.device, OUTPUT_FORMAT);
        if let Some(format) = self.surface_format() {
            let _ = self.lut_pass.pipeline_for(&self.device, format);
        }
        Ok(())
    }

    /// The bound look table's view, or `None` when the frame runs LUT-free.
    pub fn look_lut_view(&self) -> Option<&wgpu::TextureView> {
        self.lut.view()
    }

    /// True when a look table is bound: the frame then records the look pass
    /// after the blit and the display write reads its output.
    pub fn look_lut_bound(&self) -> bool {
        self.lut.applied().is_some()
    }
}

impl Presenter {
    /// Render size of a window frame. A surface frame follows the live render scale; the old
    /// half-resolution switch still wins when it is on, and a frame that stays on the device
    /// (the twin) never takes the scale.
    pub(super) fn live_render_size(&self, surface: bool, width: u32, height: u32) -> (u32, u32) {
        let legacy =
            super::live::live_present_size(width, height, super::live::live_half_res_enabled());
        if !surface || legacy != (width, height) {
            return legacy;
        }
        self.upscale.config.render_size(width, height)
    }

    /// Ready the upscale chain for this frame (and arm or disarm it). Call after the targets
    /// exist and before the frame borrows them.
    pub(super) fn prepare_upscale(&mut self, surface: bool, render: (u32, u32), out: (u32, u32)) {
        let want_final = self.look_lut_bound();
        self.upscale
            .prepare(&self.device, &self.queue, surface, render, out, want_final);
    }

    /// Take the picker's scale for this frame from the newest resolved GPU ms (a no-op unless
    /// `E2040_DYNRES=1`).
    pub(super) fn tick_dynres(&mut self) {
        let latest = self.gpu_timer.as_ref().and_then(|t| t.latest_total());
        self.upscale.tick_dynres(latest);
    }

    /// The receipt lines of the upscale chain and the dyn-res picker, `None` while both are off.
    #[must_use]
    pub fn upscale_receipt(&self) -> Option<String> {
        self.upscale.receipt_line()
    }

    /// Render the window frame at `pct` percent of the output edge (50 to 100). Below 100 the
    /// picture is stretched back to the output by the upscaler: bilinear in the blit, or the
    /// FSR 1.0 EASU pass when [`Self::set_upscale_fsr1`] is on.
    pub fn set_render_scale_pct(&mut self, pct: u8) {
        self.upscale.config.set_scale_pct(pct);
    }

    /// The live render scale in percent of the output edge.
    #[must_use]
    pub fn render_scale_pct(&self) -> u8 {
        self.upscale.config.scale_pct()
    }

    /// Switch the FSR 1.0 EASU upscale for frames that render below the output.
    pub fn set_upscale_fsr1(&mut self, on: bool) {
        self.upscale.config.set_fsr1(on);
    }

    /// True when the EASU upscale is on.
    #[must_use]
    pub fn upscale_fsr1(&self) -> bool {
        self.upscale.config.fsr1()
    }

    /// Sharpen strength after the upscale, `0` (off) to `1`.
    pub fn set_upscale_sharpness(&mut self, sharpness: f32) {
        self.upscale.config.set_sharpness(sharpness);
    }
}
