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
use super::frame::particle_err;
use super::presenter::Presenter;
use super::submit::FrameCamera;
use crate::particles_gpu::{EmitterKind, ParticlePool};

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
    /// follow the sim and a capture repeats.
    pub fn step_fight_fx(&mut self, dt: f32) {
        if let Some(pool) = self.fight_fx.as_mut() {
            pool.step(dt);
        }
    }

    /// Fight particles alive now, and spawned over the run (the receipt line).
    #[must_use]
    pub fn fight_fx_counts(&self) -> (usize, u64) {
        self.fight_fx
            .as_ref()
            .map_or((0, 0), |pool| (pool.alive(), pool.spawned_total()))
    }

    /// Stage the fight pool under this frame's camera; true when the frame has
    /// a batch to draw. A calm pool stages nothing and the frame's pixels stay
    /// a pure function of its list.
    pub(super) fn stage_fight_fx(&mut self, frame: &FrameCamera) -> Result<bool, PresentError> {
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
}
