//! warmup: build every pipeline a frame can draw with before the first frame (WEB13).
//!
//! Nearly all of the presenter's pipelines are built when it is created (forward, skinned
//! forward, sky, blit, sun depth, GTAO, the IBL chart and the compute submitters). Two were not:
//! the HP-bar overlay pipeline was built inside the frame whenever a bar was staged (one build
//! per frame, and the eight fighters of a match always stage bars), and the surface blit is built
//! when a surface is attached. [`Presenter::warm_pipelines`] builds the overlay pipeline for
//! every target format a frame can end in, and [`Presenter::overlay_pipeline_for`] keeps it so a
//! frame only looks it up. The loader then calls `pipeline_warmup::seal` and a build after that
//! is a loud `late_compile` (see [`crate::pipeline_warmup`]).
//!
//! Units: counts. Presentation only, never hashed. Caller: `crates/web` (the loader) and the
//! live and offscreen frame paths (the lookup).
use super::super::*;
use super::presenter::Presenter;

impl Presenter {
    /// The HP-bar overlay pipeline for `format`: built on the first ask, a clone of the kept one
    /// after that (a pipeline handle is a reference count). Frames call this; the warm-up makes
    /// the first ask, so a frame never builds.
    pub(super) fn overlay_pipeline_for(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let mut kept = self
            .overlay_pipes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, pipeline)) = kept.iter().find(|(f, _)| *f == format) {
            return pipeline.clone();
        }
        let pipeline = super::live::live_overlay_pipeline(&self.device, format);
        kept.push((format, pipeline.clone()));
        pipeline
    }

    /// Build the pipelines the creation did not: the overlay for the offscreen twin format and
    /// for the attached surface's format. Returns the builds this thread's ledger holds, so the
    /// caller can print `pipelines=<n>`. Idempotent. Load path only.
    pub fn warm_pipelines(&self) -> usize {
        let _ = self.overlay_pipeline_for(OUTPUT_FORMAT);
        if let Some(format) = self.surface_format() {
            let _ = self.overlay_pipeline_for(format);
        }
        crate::pipeline_warmup::snapshot().total()
    }
}

#[cfg(all(test, feature = "gpu"))]
mod tests {
    use super::*;
    use crate::pipeline_warmup as ledger;
    use crate::procedural_arena::ProceduralArena;
    use crate::test_gpu::{require_or_skip, NoAdapter};

    fn presenter_or_skip(test: &str) -> Option<Presenter> {
        ledger::reset();
        let probe = match Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(PresentError::NoAdapter) => {
                Err(NoAdapter("no wgpu adapter on this machine".into()))
            }
            Err(e) => panic!("presenter init failed (must fail loudly): {e}"),
        };
        require_or_skip(test, probe)
    }

    /// The warm-up list equals the set a frame draws: after `warm_pipelines` and the seal, a
    /// hundred arena frames (poses, HP bars staged on a third of them, MSAA on for a stretch)
    /// build nothing.
    #[test]
    fn pipeline_warmup_no_build_after_the_warmup_over_100_frames() {
        no_build_after_the_warmup(
            "pipeline_warmup_no_build_after_the_warmup_over_100_frames",
            100,
        );
    }

    /// QW7: the same bar over ten seconds of the 60 Hz loop: 600 frames, MSAA toggled every
    /// hundred, build nothing after the seal.
    #[test]
    fn pipeline_warmup_no_build_after_the_warmup_over_600_frames() {
        no_build_after_the_warmup(
            "pipeline_warmup_no_build_after_the_warmup_over_600_frames",
            600,
        );
    }

    fn no_build_after_the_warmup(test: &str, frames: u32) {
        let Some(mut p) = presenter_or_skip(test) else {
            return;
        };
        // A fresh offscreen presenter is suspended (zero area) until it has a size.
        p.resize(256, 144).expect("a valid size applies");
        assert!(!p.suspended(), "the frames below draw");
        let arena = ProceduralArena::build().expect("the stand-in builds");
        arena.stage(&mut p).expect("the arena stages");
        let built = p.warm_pipelines();
        assert!(
            built >= 8,
            "creation plus warm-up built only {built} pipelines"
        );
        let warm = ledger::seal();
        assert_eq!(warm, built, "the seal sees what the warm-up counted");
        let (w, h) = p.size();
        for frame in 0..frames {
            // MSAA on for the stretch from 40 to 70 of every hundred frames.
            match frame % 100 {
                40 => p.set_msaa_enabled(true),
                70 => p.set_msaa_enabled(false),
                _ => {}
            }
            let t = frame as f32 / 60.0;
            arena.pose(&mut p, t).expect("pose");
            p.clear_overlay();
            if frame % 3 == 0 {
                for i in 0..8u8 {
                    let x = -0.9 + f32::from(i) * 0.22;
                    assert!(p.push_hp_bar(x, 0.9, x + 0.2, 0.94, 0.6), "bar {i} fits");
                }
            }
            let list = arena.list_at(w, h, t).expect("list");
            p.render_live_offscreen(&list).expect("frame");
        }
        let after = ledger::snapshot();
        assert!(
            after.late().is_empty(),
            "pipelines built after the warm-up: {:?}",
            ledger::late_line(&after)
        );
        assert_eq!(after.total(), warm, "no build after frame 1");
        // The bars were really drawn: the overlay pipeline is in the ledger, built by the warm-up.
        assert!(
            after.labels().iter().any(|l| l == "ra1-overlay-pipeline"),
            "the overlay pipeline was never built: {:?}",
            after.labels()
        );
    }

    /// Without the warm-up the overlay pipeline is the one a frame would build; with it, a frame
    /// only looks it up (the build count does not move over repeated asks).
    #[test]
    fn pipeline_warmup_overlay_is_built_once_per_format() {
        let Some(p) = presenter_or_skip("pipeline_warmup_overlay_is_built_once_per_format") else {
            return;
        };
        let before = ledger::snapshot().total();
        let _ = p.overlay_pipeline_for(OUTPUT_FORMAT);
        let once = ledger::snapshot().total();
        assert_eq!(once, before + 1, "the first ask builds");
        for _ in 0..10 {
            let _ = p.overlay_pipeline_for(OUTPUT_FORMAT);
        }
        assert_eq!(ledger::snapshot().total(), once, "later asks reuse it");
        assert_eq!(p.warm_pipelines(), once, "warming again builds nothing");
    }
}
