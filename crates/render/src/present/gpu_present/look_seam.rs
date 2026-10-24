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
