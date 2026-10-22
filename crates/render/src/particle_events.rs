//! particle_events: the fight's event pool (FEEL5). The four emitters keep
//! their look but run no steady rate; a hit, a block or a fall throws a
//! point-sized burst where it lands. Owner: render. Units: meters, world
//! space. Determinism: same seed, same calls, same bytes. Budget: one burst
//! is at most [`MAX_BURST`] spawns, inside the pool's tier caps.

use super::{EmitterKind, ParticleError, ParticlePool, EMITTER_COUNT};

/// Most particles one fight event may throw (FEEL5); bigger asks clamp.
pub const MAX_BURST: u32 = 256;
/// Half-extent of a burst's spawn box in meters: an event happens at a point
/// (a blade on a shield, a cut, a body hitting the sand), so the emitter's
/// own wide box (dust spans 1.5 m) is cut down to this.
pub const BURST_EXTENT_M: f32 = 0.12;

impl ParticlePool {
    /// The fight's pool (FEEL5): the four emitters keep their look (life,
    /// speed, gravity, drag, size, tint) but run no steady rate and no
    /// tick-zero burst, so nothing spawns until an event calls
    /// [`Self::burst_at`]. The cap is the tier's, like every pool.
    pub fn events(seed: u64, cap: usize) -> Result<Self, ParticleError> {
        let mut pool = Self::with_capacity(seed, cap)?;
        for emitter in &mut pool.emitters {
            emitter.rate_per_s = 0.0;
            emitter.burst = 0;
        }
        Ok(pool)
    }

    /// Throw up to `count` particles of `kind` from `origin_m` along `dir`
    /// (meters, world space; the emitter's spread jitters around it) and
    /// return how many spawned. The kind's look is unchanged; only the spawn
    /// point, direction and a point-sized box are this event's. A tier or
    /// pool cap stops the burst at the first refusal (counted once as a
    /// drop); a non-finite point or direction throws nothing. Same seed,
    /// same calls, same bytes.
    pub fn burst_at(
        &mut self,
        kind: EmitterKind,
        origin_m: [f32; 3],
        dir: [f32; 3],
        count: u32,
    ) -> u32 {
        if !origin_m.iter().chain(dir.iter()).all(|v| v.is_finite()) {
            return 0;
        }
        let slot = kind.index().min(EMITTER_COUNT - 1);
        let own = self.emitters[slot];
        let emitter = &mut self.emitters[slot];
        emitter.origin_m = origin_m;
        emitter.dir = dir;
        emitter.extent_m = own.extent_m.map(|e| e.min(BURST_EXTENT_M));
        let mut spawned = 0;
        for _ in 0..count.min(MAX_BURST) {
            if self.spawn_at(slot).is_err() {
                break;
            }
            spawned += 1;
        }
        self.emitters[slot] = own;
        spawned
    }
}

#[cfg(test)]
mod tests {
    use super::super::PoolEmitter;
    use super::*;

    /// FEEL5: the fight's pool is quiet until an event, a burst lands at
    /// the event's point and flies its way, the tier cap stops it, and the
    /// same calls give the same bytes.
    #[test]
    fn an_event_pool_is_quiet_until_a_burst_lands_where_it_was_thrown() {
        let mut pool = ParticlePool::events(7, 512).expect("an event pool builds");
        for _ in 0..30 {
            pool.tick();
        }
        assert_eq!(pool.alive(), 0, "no steady rate and no tick-zero burst");
        let at = [3.0, 1.25, -2.0];
        let thrown = pool.burst_at(EmitterKind::Sparks, at, [1.0, 0.3, 0.0], 20);
        assert_eq!(thrown, 20);
        for p in pool.particles() {
            for (got, want) in p.pos.iter().zip(at) {
                assert!(
                    (got - want).abs() <= BURST_EXTENT_M + 1e-6,
                    "spawned inside the point-sized box, got {:?}",
                    p.pos
                );
            }
        }
        let mean_vx: f32 = pool.particles().iter().map(|p| p.vel[0]).sum::<f32>() / 20.0;
        assert!(mean_vx > 0.5, "the burst flies along +x, mean vx {mean_vx}");
        assert_eq!(
            pool.emitters()[EmitterKind::Sparks.index()],
            PoolEmitter {
                rate_per_s: 0.0,
                burst: 0,
                ..PoolEmitter::sparks()
            },
            "the emitter's own box and direction come back after the burst"
        );
        let cap = pool.kind_caps().of(EmitterKind::Blood);
        let thrown = pool.burst_at(EmitterKind::Blood, at, [0.0, 1.0, 0.0], cap + 50);
        assert_eq!(thrown, cap, "the tier cap stops the burst");
        assert_eq!(pool.dropped_total(), 1, "one refusal, counted once");
        assert_eq!(
            pool.burst_at(EmitterKind::Dust, [f32::NAN, 0.0, 0.0], [0.0, 1.0, 0.0], 5),
            0
        );
        let mut twin = ParticlePool::events(7, 512).expect("builds");
        for _ in 0..30 {
            twin.tick();
        }
        twin.burst_at(EmitterKind::Sparks, at, [1.0, 0.3, 0.0], 20);
        twin.burst_at(EmitterKind::Blood, at, [0.0, 1.0, 0.0], cap + 50);
        assert_eq!(
            pool.particle_bytes(),
            twin.particle_bytes(),
            "same calls, same bytes"
        );
        for _ in 0..120 {
            pool.tick();
        }
        assert_eq!(pool.alive(), 0, "a burst dies out within its life");
    }
}
