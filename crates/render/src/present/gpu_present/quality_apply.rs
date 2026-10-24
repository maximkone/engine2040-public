//! quality_apply: a quality tier's knobs reaching the live frame (WEB8 gap).
//!
//! Owner: render lane. Units: texels, slots, counts. The pure decisions (`shadow_edge_px`,
//! `max_cascade_band`, `gtao_runs`, `particle_pool_slots`, `decal_pool_slots`) live in
//! `crate::quality`; this file applies them to the presenter's resources. Presentation only,
//! never hashed.

use super::presenter::Presenter;
use crate::quality::{self, Knobs};
use crate::{decals, particles_gpu};

/// Seed of the live particle pool, kept across a tier change.
pub(super) const POOL_SEED: u64 = 2040;

impl Presenter {
    /// Apply a tier to the live frame: the sun shadow map is rebuilt at the tier's edge, the
    /// particle and decal pools at the tier's caps; the cascade band, the AO pass and the
    /// texture cap are read from the stored knobs by every later frame and upload (textures
    /// already resident keep the size they were loaded at). The render scale goes to the live
    /// upscaler (a tier under 100 turns the EASU pass on, unless a run pinned it with
    /// `E2040_RENDER_SCALE` / `E2040_UPSCALE`); bloom stays the page's own.
    pub fn apply_quality(&mut self, knobs: &Knobs) {
        let max = self.device.limits().max_texture_dimension_2d;
        self.sun_sub.resize(quality::shadow_edge_px(knobs, max));
        // SW1: the tier's cascade layers and periods, capped at the layers the
        // forward pass samples so no layer draws unread.
        self.sun_sub.set_cascades(
            (quality::max_cascade_band(knobs) + 1).min(super::sun_shadow::SUN_LAYERS_READ),
            quality::cascade_periods(knobs),
        );
        self.particles = particles_gpu::ParticlePool::with_capacity(
            POOL_SEED,
            quality::particle_pool_slots(knobs),
        )
        .unwrap_or_else(|_| particles_gpu::ParticlePool::new(POOL_SEED));
        self.fight_fx = None; // FEEL5: the next event rebuilds it at this tier's cap
        self.decal_pool = decals::DecalPool::with_capacity(quality::decal_pool_slots(knobs));
        // QL5: the look guard preset is a pipeline constant, so only a tier
        // that moves it rebuilds the four forward pipelines (the potato edge).
        if quality::look_potato_guard(knobs) != quality::look_potato_guard(&self.quality) {
            let constants = quality::look_guard_constants(knobs);
            let [hdr, hdr_msaa, skinned, skinned_msaa] = super::presenter::forward_pipelines(
                &self.device,
                &self.forward_shader,
                &self.forward_layout,
                &self.ao_layout,
                &constants,
            );
            self.forward_hdr = hdr;
            self.forward_hdr_msaa = hdr_msaa;
            self.forward_skinned = skinned;
            self.forward_skinned_msaa = skinned_msaa;
            // LF3 landing 2: the depth prepass pipelines carry the same
            // preset, so they move with the forward twins.
            let [depth, depth_skinned] = super::submit::forward_depth_pipelines(
                &self.device,
                &self.forward_shader,
                &self.forward_layout,
                &constants,
            );
            self.depth_pipeline = depth;
            self.depth_skinned_pipeline = depth_skinned;
        }
        self.upscale.config.apply_tier_scale(knobs.render_scale_pct);
        self.quality = *knobs;
    }

    /// The knobs `apply_quality` last stored (the top tier before any call).
    #[must_use]
    pub fn quality_knobs(&self) -> &Knobs {
        &self.quality
    }

    /// Edge of the live sun shadow map in texels.
    #[must_use]
    pub fn shadow_map_edge(&self) -> u32 {
        self.sun_sub.edge()
    }

    /// Slots of the live particle pool.
    #[must_use]
    pub fn particle_slots(&self) -> usize {
        self.particles.cap()
    }

    /// Slots of the live decal pool.
    #[must_use]
    pub fn decal_slots(&self) -> usize {
        self.decal_pool.capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::present::{Presenter, TextureUpload};
    use crate::quality::{knobs, Quality};
    use crate::test_gpu::{require_or_skip, NoAdapter};

    fn presenter_or_skip(test: &str) -> Option<Presenter> {
        let probe = match Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(crate::present::PresentError::NoAdapter) => {
                Err(NoAdapter("no wgpu adapter on this machine".into()))
            }
            Err(e) => panic!("presenter init failed (must fail loudly): {e}"),
        };
        require_or_skip(test, probe)
    }

    #[test]
    fn quality_applied_shadow_map_and_pools_follow_the_tier() {
        let Some(mut presenter) = presenter_or_skip("quality_applied_shadow_map_and_pools") else {
            return;
        };
        let max = presenter.device.limits().max_texture_dimension_2d;
        for tier in [Quality::Potato, Quality::Web, Quality::Comfort] {
            let k = knobs(tier);
            presenter.apply_quality(&k);
            assert_eq!(
                presenter.shadow_map_edge(),
                quality::shadow_edge_px(&k, max),
                "{tier:?} shadow edge"
            );
            assert_eq!(presenter.particle_slots(), quality::particle_pool_slots(&k));
            assert_eq!(presenter.decal_slots(), quality::decal_pool_slots(&k));
        }
        presenter.apply_quality(&knobs(Quality::Potato));
        assert_eq!(presenter.shadow_map_edge(), 1024);
        assert_eq!(presenter.particle_slots(), 512);
        assert_eq!(presenter.decal_slots(), 64);
        assert_eq!(presenter.quality_knobs().shadow_cascades, 1);
    }

    #[test]
    fn quality_applied_texture_cap_loads_a_smaller_mip() {
        let Some(mut presenter) = presenter_or_skip("quality_applied_texture_cap") else {
            return;
        };
        let upload = TextureUpload {
            width: 2048,
            height: 1024,
            rgba: vec![200; 2048 * 1024 * 4],
        };
        let (open, _) = presenter.pack_texture(&upload).expect("uncapped upload");
        assert_eq!((open.width, open.height), (2048, 1024));
        presenter.apply_quality(&knobs(Quality::Web));
        let (capped, _) = presenter.pack_texture(&upload).expect("capped upload");
        assert_eq!(
            (capped.width, capped.height),
            (1024, 512),
            "the web tier keeps 1024 texels on the long edge"
        );
        presenter.apply_quality(&knobs(Quality::Potato));
        let (small, _) = presenter.pack_texture(&upload).expect("potato upload");
        assert_eq!((small.width, small.height), (512, 256));
    }
}
