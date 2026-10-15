use std::fmt;

use super::surface::PresentError;

// ---------------------------------------------------------------------------
// RP6: TAA/SMAA selection + Q1/RA8 manifest wiring.
//
// Owner: render lane; consumer WIN1 window loop (plus ED1/Q1 on the same
// shape). Units: pixels for sizes, scale is a render/present ratio.
// Determinism: selection is a pure function of the RP5 controller state,
// the TAA-off compile reuses the frozen `post::compile_taa_off_for_scale`
// path, and the shimmer A/B reruns bit-identical; wall clock feeds
// receipts only. Errors: `compile_*` maps `PostError` into `PresentError`,
// manifest gaps fail as `ManifestError` naming the field. Budget: selection
// is O(1) with no allocation; the compile allocates its RA2 plan only;
// the stamp is a cold receipt path, never per-draw.
// ---------------------------------------------------------------------------

/// Which AA path the presenter runs this frame. TAA is the default;
/// the SMAA fallback (RP4 TAA-off graph at the controller scale) holds
/// 60 fps under load.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AaChoice {
    /// Full-rate temporal AA (RP1 resolve + TAAU).
    Taa,
    /// SMAA fallback + dynamic resolution (RP4 TAA-off graph).
    SmaaFallback,
}

impl AaChoice {
    /// Manifest label quoted on every RA8 capture (`taa` / `smaa`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Taa => "taa",
            Self::SmaaFallback => "smaa",
        }
    }
}

/// Select by the RP5 controller state: full-rate presents run TAA, any
/// stepped-down scale runs the SMAA fallback at the controller size.
/// Pure function of the controller; the `aa_scale` API stays frozen
/// (read-only `scale` + `policy`).
#[must_use]
pub fn select_aa(controller: &crate::aa_scale::ScaleController) -> AaChoice {
    if controller.scale() >= controller.policy().max_scale {
        AaChoice::Taa
    } else {
        AaChoice::SmaaFallback
    }
}

fn post_err(context: &str, e: crate::post::PostError) -> PresentError {
    match e {
        crate::post::PostError::Invalid(reason) => PresentError::InvalidDraw {
            index: 0,
            reason: format!("rp6 {context}: {reason}"),
        },
        crate::post::PostError::BudgetExceeded { requested, allowed } => {
            PresentError::InvalidDraw {
                index: 0,
                reason: format!("rp6 {context}: budget {requested} over {allowed}"),
            }
        }
        crate::post::PostError::Device(reason) => {
            PresentError::Device(format!("rp6 {context}: {reason}"))
        }
        crate::post::PostError::Validation(reason) => {
            PresentError::Validation(format!("rp6 {context}: {reason}"))
        }
    }
}

/// Production TAA-off compile at the controller scale: the RP5 seam
/// (`post::compile_taa_off_for_scale` over the frozen
/// `aa_scale::compile_taa_off_for_scale`) sizes the render target from
/// the controller hysteresis, then re-checks the SMAA-first order.
/// Returns the compiled graph plus the render size it was built at, so
/// the window build renders the TAA-off path at the controller scale.
/// Errors map into [`PresentError`]; fails loudly on bad sizes.
pub fn compile_taa_off_at_controller_scale(
    controller: &crate::aa_scale::ScaleController,
    present_w: u32,
    present_h: u32,
    chain: &crate::post::PostChain,
) -> Result<(crate::graph::CompiledGraph, u32, u32), PresentError> {
    crate::post::compile_taa_off_for_scale(controller, present_w, present_h, chain)
        .map_err(|e| post_err("taa-off compile", e))
}

/// RA8/Q1 manifest for one presented frame: the RA8 capture identity
/// (adapter, driver, resolution, quality tier, scene digest) plus the
/// RP6 stamp (AA choice + controller scale + render size). The choice
/// and scale come from the live controller, so the manifest can never
/// disagree with the frame that was selected.
#[derive(Clone, PartialEq, Debug)]
pub struct AaFrameManifest {
    /// RA8 capture identity (adapter/driver/resolution/quality/digest).
    pub capture: crate::manifest::CaptureManifest,
    /// AA path selected for this frame.
    pub aa: AaChoice,
    /// Controller render/present ratio behind this frame.
    pub scale: f32,
    /// Render width in pixels the frame was built at.
    pub render_w: u32,
    /// Render height in pixels the frame was built at.
    pub render_h: u32,
}

impl AaFrameManifest {
    /// Stamp one frame: builds the RA8 capture over the present size,
    /// then stamps the choice + scale from `controller` with the render
    /// size the graph was compiled at. Empty adapter/driver, zero sizes
    /// and non-hex digests fail as [`crate::manifest::ManifestError`]
    /// naming the field; a suspended (zero-area) frame cannot stamp.
    #[expect(
        clippy::too_many_arguments,
        reason = "wide signature kept until its inputs move into a params struct"
    )]
    pub fn new(
        adapter: &str,
        driver: &str,
        present_w: u32,
        present_h: u32,
        quality: crate::manifest::QualityTier,
        scene_digest: &str,
        controller: &crate::aa_scale::ScaleController,
        render_w: u32,
        render_h: u32,
    ) -> Result<Self, crate::manifest::ManifestError> {
        if render_w == 0 || render_h == 0 {
            return Err(crate::manifest::ManifestError::InvalidResolution {
                width_px: render_w,
                height_px: render_h,
            });
        }
        let capture = crate::manifest::CaptureManifest::new(
            adapter,
            driver,
            present_w,
            present_h,
            quality,
            scene_digest,
        )?;
        Ok(Self {
            capture,
            aa: select_aa(controller),
            scale: controller.scale(),
            render_w,
            render_h,
        })
    }

    /// Fixed-order one-line summary for capture logs: the RA8 fields in
    /// manifest order, then the RP6 stamp (`aa`, `scale`, `render`).
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{} aa={} scale={:.3} render={}x{}",
            self.capture.summary(),
            self.aa.name(),
            self.scale,
            self.render_w,
            self.render_h
        )
    }
}

/// Shimmer A/B variances over the RP1 checkerboard pair: raw flicker,
/// the TAA leg (mid-grey history, 12 alternating frames, settled tail,
/// the honest presenter path from the RP1 gate) and the SMAA leg (the
/// RP4 fallback over each frame). Cold oracle path for Q1 evidence,
/// never per-frame. Same inputs give bit-identical outputs.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShimmerAb {
    /// Raw pair variance (posts ~0.16).
    pub raw: f32,
    /// TAA settled variance (posts under `post::SHIMMER_MAX_VAR` 0.01).
    pub taa: f32,
    /// SMAA settled variance (posts under `SMAA_SHIMMER_MAX_VAR` 0.06).
    pub smaa: f32,
}

fn ab_err(context: &str, e: impl fmt::Display) -> PresentError {
    PresentError::InvalidDraw {
        index: 0,
        reason: format!("rp6 shimmer {context}: {e}"),
    }
}

/// Run the shimmer A/B and return all three variances for the Q1 post.
/// Fails loudly when the scene, the TAA leg or the SMAA leg refuses.
pub fn shimmer_ab() -> Result<ShimmerAb, PresentError> {
    use crate::{aa_scale, post};
    let scene = post::ShimmerScene::checkerboard().map_err(|e| ab_err("scene", e))?;
    let raw = post::ShimmerScene::temporal_variance(&scene.frames);
    let motion =
        post::MotionField::zero(scene.width, scene.height).map_err(|e| ab_err("motion", e))?;
    let taa_params = post::TaaParams::default();
    let mut history = vec![[0.5, 0.5, 0.5]; scene.width as usize * scene.height as usize];
    let mut clip = Vec::with_capacity(12);
    for k in 0..12 {
        let current = &scene.frames[k % 2];
        let resolved = post::taa_resolve(
            current,
            &history,
            &motion,
            scene.width,
            scene.height,
            &taa_params,
        )
        .map_err(|e| ab_err("taa", e))?;
        clip.push(resolved.clone());
        history = resolved;
    }
    let taa = post::ShimmerScene::temporal_variance(&clip[8..]);
    let smaa_params = aa_scale::SmaaParams::default();
    let mut settled = Vec::with_capacity(scene.frames.len());
    for frame in &scene.frames {
        settled.push(
            aa_scale::smaa_resolve(frame, scene.width, scene.height, &smaa_params)
                .map_err(|e| ab_err("smaa", e))?,
        );
    }
    let smaa = post::ShimmerScene::temporal_variance(&settled);
    Ok(ShimmerAb { raw, taa, smaa })
}
