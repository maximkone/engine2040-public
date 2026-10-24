//! outline_hook: SW150's one wiring point in the frame. The presenter owns the
//! chain and the locked target; `encode_forward` calls [`Presenter::encode_outline`]
//! after it wrote the frame's depth, and `ensure_targets` calls
//! [`Presenter::ensure_outline`] on the resize that re-sizes the frame targets.
//! A frame with no locked target records nothing, and the potato tier never
//! builds a chain, so an unlocked frame pays no draw.
//!
//! Units: metres and texels. Presentation only: the pass reads the frame's depth
//! and writes the HDR band, and nothing here is hashed.

use super::frame::Targets;
use super::Presenter;
use crate::present::DrawList;

impl Presenter {
    /// SW150: (re)build the outline chain when the frame size or the quality
    /// tier changed; `None` on the potato tier, which draws no outline. Both the
    /// live and the capture path reach it through `ensure_targets`.
    pub(super) fn ensure_outline(&mut self, width: u32, height: u32) {
        let plan = crate::outline::outline_plan(tier(self), width, height, 1);
        let stale = self
            .outline
            .as_ref()
            .is_none_or(|chain| chain.mask_size() != plan.mask_size);
        if !plan.enabled || !stale {
            return;
        }
        self.outline =
            crate::outline::gpu::OutlineGpu::new(&self.device, tier(self), width, height);
        if let Some(chain) = self.outline.as_ref() {
            chain.upload_mesh(&self.queue);
        }
    }

    /// SW150: encode the locked target's outline over `targets`' depth: the mask
    /// at half res, the Sobel edge in the visible or hidden colour, the two
    /// blurs and the composite. Nothing happens until a target is locked.
    pub(super) fn encode_outline(
        &self,
        enc: &mut wgpu::CommandEncoder,
        targets: &Targets,
        list: &DrawList,
    ) {
        let (Some(chain), Some(target)) = (self.outline.as_ref(), self.outline_target) else {
            return;
        };
        let camera = &list.camera;
        // The frame's own aspect wins over the camera's: the mask must cover the
        // target the size the frame targets have.
        let aspect = frame_aspect(targets.width, targets.height);
        let Some(view) = crate::outline::OutlineView::new(
            [camera.eye.x, camera.eye.y, camera.eye.z],
            [camera.target.x, camera.target.y, camera.target.z],
            camera.fov_y,
            aspect,
            camera.near,
            camera.far,
        ) else {
            return;
        };
        chain.encode(
            &self.device,
            &self.queue,
            enc,
            &crate::outline::gpu::OutlineFrame {
                view: &view,
                prefill: &targets.depth_view,
                prefill_size: [targets.width, targets.height],
                targets: &[target],
            },
        );
    }

    /// Lock (or clear) the fighter the outline draws: the render side's one
    /// input for SW150. `None` records no outline on the next frame; a lock
    /// after a tier change builds that tier's chain, and the potato tier builds
    /// none.
    pub fn set_outline_target(&mut self, target: Option<crate::outline::OutlineTarget>) {
        self.outline_target = target;
        let (width, height) = self.swap.size();
        self.ensure_outline(width, height);
    }
}

/// The frame's width over height, the aspect the mask projects with; a zero
/// height has no frame and the caller skips the pass.
#[must_use]
#[expect(clippy::cast_precision_loss, reason = "frame sides fit f32")]
fn frame_aspect(width: u32, height: u32) -> f32 {
    if height == 0 {
        return 1.0;
    }
    width as f32 / height as f32
}

/// The quality tier the presenter's live knobs stand for.
#[must_use]
fn tier(presenter: &Presenter) -> crate::quality::Quality {
    crate::outline::tier_of_knobs(&presenter.quality)
}
