//! fight_fx: the fight's own particles in the live frame and its capture (FEEL5).
//!
//! Owner: render lane. The probe pool (`Presenter::particles`) is the look row's
//! steady demo fountain and stays a probe-frame draw. This pool is the game's:
//! quiet until a fight event throws a burst (`fight_burst`), stepped by the game
//! once per sim tick (`step_fight_fx`), staged and drawn by every live and
//! blocking frame while anything is alive. Because the game steps it and the
//! frame only stages it, a capture twin sees the same pool twice and repeats
//! byte-exact. The cap is the quality tier's particle budget. Presentation only,
//! never hashed. Units: meters, seconds.

use super::super::PresentError;
use super::frame::{particle_err, trail_err};
use super::presenter::Presenter;
use super::submit::FrameCamera;
use crate::particles_gpu::{EmitterKind, ParticlePool};
use crate::vfx_soft::{vfx_light, vfx_zparams};

/// Seed of the fight pool: one stream for the run, so the same events give the
/// same particles.
const FIGHT_FX_SEED: u64 = 0xFEE1_0005;

impl Presenter {
    /// Throw up to `count` particles of `kind` at `at_m` along `dir` (world
    /// meters) and return how many spawned. The pool is built on the first
    /// event at the tier's particle cap; the cap stops a burst early.
    pub fn fight_burst(
        &mut self,
        kind: EmitterKind,
        at_m: [f32; 3],
        dir: [f32; 3],
        count: u32,
    ) -> u32 {
        if self.fight_fx.is_none() {
            // Never the probe pool as a stand-in: its steady emitters are a
            // fountain at the ring centre. No pool means no particles.
            self.fight_fx = ParticlePool::events(FIGHT_FX_SEED, self.particles.cap()).ok();
        }
        self.fight_fx
            .as_mut()
            .map_or(0, |pool| pool.burst_at(kind, at_m, dir, count))
    }

    /// Advance the fight's particles by one sim tick of `dt` seconds. The game
    /// calls this once per tick it ran, never per presented frame, so pixels
    /// follow the sim and a capture repeats. The ground splats age on the same
    /// tick, so blood dries with the fight.
    pub fn step_fight_fx(&mut self, dt: f32) {
        if let Some(pool) = self.fight_fx.as_mut() {
            pool.step(dt);
        }
        self.decal_pool.tick();
    }

    /// Fight particles alive now, and spawned over the run (the receipt line).
    #[must_use]
    pub fn fight_fx_counts(&self) -> (usize, u64) {
        self.fight_fx
            .as_ref()
            .map_or((0, 0), |pool| (pool.alive(), pool.spawned_total()))
    }

    /// Fight spawns per [`EmitterKind::ALL`] slot over the run (dust, flame,
    /// sparks, blood): the receipt counts each cue's particles apart.
    #[must_use]
    pub fn fight_fx_by_kind(&self) -> [u64; 4] {
        self.fight_fx
            .as_ref()
            .map_or([0; 4], |pool| pool.spawned_by_kind())
    }

    /// Leave a blood splat on the floor at `at_m` (world meters, on the
    /// ground) with `radius_m`; false when the point or radius is refused.
    /// The field is the tier's decal cap, so a full field drops its oldest
    /// blood instead of growing.
    pub fn fight_splat(&mut self, at_m: [f32; 3], radius_m: f32) -> bool {
        self.decal_pool.splat_at(at_m, radius_m).is_ok()
    }

    /// Ground splats: resident, placed over the run, evicted as oldest, and
    /// the tier's splat cap.
    #[must_use]
    pub fn fight_splat_counts(&self) -> (usize, u64, u64, usize) {
        let (resident, placed, evicted) = self.decal_pool.splat_counts();
        (resident, placed, evicted, self.decal_pool.splat_capacity())
    }

    /// Stage the ground splats under this frame's camera; true when the frame
    /// has splats to draw. Every call stages, so a field that emptied stops
    /// drawing instead of repeating the last frame's blood.
    pub(super) fn stage_fight_splats(&mut self, frame: &FrameCamera) -> Result<bool, PresentError> {
        let staged = self
            .decal_sub
            .stage_splats(
                &self.device,
                &self.queue,
                self.decal_pool.splats(),
                frame.built_view_proj.to_cols_array_2d(),
            )
            .map_err(|error| PresentError::Validation(error.to_string()))?;
        Ok(staged > 0)
    }

    /// Stage the fight pool under this frame's camera; true when the frame has
    /// a batch to draw. A calm pool stages nothing and the frame's pixels stay
    /// a pure function of its list.
    pub(super) fn stage_fight_fx(&mut self, frame: &FrameCamera) -> Result<bool, PresentError> {
        self.set_vfx_camera(frame);
        let Some(pool) = self.fight_fx.as_ref().filter(|pool| pool.alive() > 0) else {
            return Ok(false);
        };
        let receipt = self
            .particle_sub
            .stage_pool(
                &self.device,
                &self.queue,
                pool,
                frame.built_view_proj.to_cols_array_2d(),
                frame.camera_right(),
                frame.camera_up(),
            )
            .map_err(particle_err)?;
        Ok(receipt.staged > 0)
    }

    /// Hand the sprite and ribbon passes the frame camera's view-depth
    /// constants (they turn the stored depth back into metres for the soft
    /// contact). Called before either stages, every frame that stages.
    pub(super) fn set_vfx_camera(&mut self, frame: &FrameCamera) {
        let mut vfx = self.particle_sub.vfx();
        vfx.zparams = vfx_zparams(frame.near, frame.far);
        self.particle_sub.set_vfx(&self.queue, vfx);
        self.trail_sub.set_vfx(&self.queue, vfx);
    }

    /// Hand the sprite and ribbon passes the light of this frame: the sun's
    /// radiance from the frame's light uniform bytes (`LightU`: direction, pad,
    /// `light_color`, intensity) plus the sky's mean radiance, so dust and blood
    /// take the arena's light and the streak its cast. A short or non-finite
    /// image keeps the last light.
    pub(super) fn set_vfx_light(&mut self, light_bytes: &[u8]) {
        let word = |at: usize| {
            light_bytes
                .get(at..at + 4)
                .and_then(|b| <[u8; 4]>::try_from(b).ok())
                .map(f32::from_le_bytes)
        };
        let (Some(r), Some(g), Some(b)) = (word(16), word(20), word(24)) else {
            return;
        };
        let mut vfx = self.particle_sub.vfx();
        vfx.light = vfx_light([r, g, b], self.ibl_ambient());
        if self.particle_sub.set_vfx(&self.queue, vfx) {
            self.trail_sub.set_vfx(&self.queue, vfx);
        }
    }

    /// Stage the sword ribbon under this frame's camera. The probe and capture
    /// frames stage it with the look passes; the live frame has no look pass, so
    /// it stages here. An empty list clears the last strip.
    pub(super) fn stage_fight_trail(&mut self, frame: &FrameCamera) -> Result<(), PresentError> {
        self.trail_sub
            .stage(
                &self.device,
                &self.queue,
                &self.trail_verts,
                &self.trail_lens,
                frame.built_view_proj.to_cols_array_2d(),
                frame.camera_right(),
                frame.camera_up(),
            )
            .map(|_| ())
            .map_err(trail_err)
    }
}
