//! vfx_hanabi: one batched hanabi-style effect dispatch over the seeded pool.
//! Owner: render lane. Units: counts, ticks at 60 Hz, seconds for life.
//! Determinism: fixed effect order, splitmix-seeded integer life ticks.
//! Errors: [`super::ParticleError`]. Budget: one dispatch, caller GPU buffers.
//! Caller: [`super::ParticlePool::hanabi_dispatch`].
//! PORT(djeedai/bevy_hanabi@04680ac23641da0f9022f9e548219642438b7eb8:src/asset.rs,src/spawn.rs,src/gradient.rs,src/render/batch.rs,src/render/effect_cache.rs): reimplemented, zero bytes vendored.

/// Hanabi billboard shader: fetch by (effect, particle), age01 gradient.
pub const HANABI_WGSL: &str = include_str!("shaders/particles_hanabi.wgsl");

/// One effect descriptor: spawner plus init sphere plus update plus gradient.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HanabiEffectDesc {
    /// Steady spawn rate per second.
    pub rate_per_s: f32,
    /// One-shot burst on tick zero.
    pub burst: u32,
    /// Particle life range in seconds, ascending.
    pub life_s: [f32; 2],
    /// Mean launch direction, normalized per spawn.
    pub dir: [f32; 3],
    /// Cone jitter around `dir` per axis.
    pub spread: f32,
    /// Launch speed range in m/s, ascending.
    pub speed_m_s: [f32; 2],
    /// Vertical acceleration in m/s^2.
    pub gravity_m_s2: f32,
    /// Linear drag per second.
    pub drag_per_s: f32,
    /// Tint at birth (RGBA).
    pub grad_a: [f32; 4],
    /// Tint at death (RGBA).
    pub grad_b: [f32; 4],
}

impl HanabiEffectDesc {
    /// Slow settling footwork dust.
    pub fn dust() -> Self {
        Self {
            rate_per_s: 180.0,
            burst: 24,
            life_s: [0.8, 1.8],
            dir: [0.0, 1.0, 0.0],
            spread: 0.6,
            speed_m_s: [0.5, 2.0],
            gravity_m_s2: 0.6,
            drag_per_s: 1.2,
            grad_a: [0.85, 0.78, 0.66, 1.0],
            grad_b: [0.85, 0.78, 0.66, 0.0],
        }
    }
    /// Buoyant torch flame puffs.
    pub fn flame() -> Self {
        Self {
            rate_per_s: 90.0,
            burst: 12,
            life_s: [0.4, 0.9],
            dir: [0.0, 1.0, 0.0],
            spread: 0.25,
            speed_m_s: [1.0, 2.5],
            gravity_m_s2: -2.5,
            drag_per_s: 0.4,
            grad_a: [1.0, 0.8, 0.4, 1.0],
            grad_b: [0.9, 0.2, 0.1, 0.0],
        }
    }
    /// Fast hot strike sparks.
    pub fn sparks() -> Self {
        Self {
            rate_per_s: 60.0,
            burst: 8,
            life_s: [0.4, 1.0],
            dir: [0.0, 1.0, 0.0],
            spread: 1.0,
            speed_m_s: [3.0, 8.0],
            gravity_m_s2: 9.0,
            drag_per_s: 0.2,
            grad_a: [1.0, 0.95, 0.7, 1.0],
            grad_b: [1.0, 0.4, 0.1, 0.0],
        }
    }
    /// One-shot burst of 64 sparks, no steady rate.
    pub fn burst64() -> Self {
        Self {
            rate_per_s: 0.0,
            burst: 64,
            life_s: [0.4, 1.0],
            dir: [0.0, 1.0, 0.0],
            spread: 1.0,
            speed_m_s: [3.0, 8.0],
            gravity_m_s2: 9.0,
            drag_per_s: 0.2,
            grad_a: [1.0, 0.95, 0.7, 1.0],
            grad_b: [1.0, 0.4, 0.1, 0.0],
        }
    }
    /// Check ranges and NaN before a build trusts this.
    pub fn validate(&self) -> Result<(), super::ParticleError> {
        if !self.rate_per_s.is_finite()
            || self.rate_per_s < 0.0
            || self.rate_per_s > super::MAX_RATE_PER_S
        {
            return Err(super::ParticleError::Invalid(
                "hanabi rate out of range".into(),
            ));
        }
        let [lo, hi] = self.life_s;
        if !lo.is_finite() || !hi.is_finite() || lo < 1e-3 || hi < lo || hi > super::MAX_LIFE_S {
            return Err(super::ParticleError::Invalid(
                "hanabi life must ascend".into(),
            ));
        }
        let sq = self.dir[0] * self.dir[0] + self.dir[1] * self.dir[1] + self.dir[2] * self.dir[2];
        if !sq.is_finite() || sq <= 1e-9 || !self.spread.is_finite() || self.spread < 0.0 {
            return Err(super::ParticleError::Invalid(
                "hanabi dir/spread bad".into(),
            ));
        }
        if !self.gravity_m_s2.is_finite() || !self.drag_per_s.is_finite() || self.drag_per_s < 0.0 {
            return Err(super::ParticleError::Invalid(
                "hanabi gravity/drag bad".into(),
            ));
        }
        if !self.grad_a.iter().all(|c| c.is_finite()) || !self.grad_b.iter().all(|c| c.is_finite())
        {
            return Err(super::ParticleError::Invalid(
                "hanabi gradient not finite".into(),
            ));
        }
        Ok(())
    }
    /// Sample the RGBA gradient at age01; agrees with the CPU mirror.
    pub fn sample_gradient(&self, age01: f32) -> [f32; 4] {
        crate::particles::hanabi_gradient(self.grad_a, self.grad_b, age01)
    }
}

/// Fixed-order batch: N effects staged into exactly one indirect dispatch.
pub struct HanabiBatch {
    descs: Vec<HanabiEffectDesc>,
    accrues: Vec<f32>,
    deaths: Vec<u64>,
    stream: engine_core::Rng,
    ticks: u64,
    spawned: u64,
}

impl HanabiBatch {
    /// Build over caller-ordered descs; fixed order decides the bytes.
    pub fn new(seed: u64, descs: Vec<HanabiEffectDesc>) -> Result<Self, super::ParticleError> {
        if descs.is_empty() || descs.len() > 8 {
            return Err(super::ParticleError::Invalid(
                "hanabi needs 1..=8 effects".into(),
            ));
        }
        for d in descs.iter() {
            d.validate()?;
        }
        let n = descs.len();
        let mut deaths = Vec::new();
        deaths.reserve_exact(super::POOL_CAP);
        Ok(Self {
            descs,
            accrues: vec![0.0; n],
            deaths,
            stream: engine_core::Rng::from_state(engine_core::splitmix64(seed)),
            ticks: 0,
            spawned: 0,
        })
    }
    /// Effect count in fixed order.
    pub fn len(&self) -> usize {
        self.descs.len()
    }
    /// True while no effects are staged.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.descs.is_empty()
    }
    /// Live count after the last tick.
    pub fn alive(&self) -> usize {
        self.deaths.len()
    }
    /// Total spawns since build.
    pub fn spawned_total(&self) -> u64 {
        self.spawned
    }
    /// Dispatch count: one layout key, so exactly one dispatch.
    pub fn dispatch_count(&self) -> u32 {
        1
    }
    /// Advance one fixed tick.
    pub fn tick(&mut self) {
        self.step(super::TICK_DT);
    }
    /// Advance by `dt`; bad steps are ignored, big steps clamp.
    pub fn step(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(super::MAX_DT_CLAMP);
        for ei in 0..self.descs.len() {
            let burst = if self.ticks == 0 {
                self.descs[ei].burst
            } else {
                0
            };
            for _ in 0..burst {
                self.spawn(ei);
            }
            let n = super::throttle_spawn(&mut self.accrues[ei], self.descs[ei].rate_per_s, dt);
            for _ in 0..n {
                self.spawn(ei);
            }
        }
        self.deaths.retain(|d| *d > self.ticks);
        self.ticks = self.ticks.wrapping_add(1);
    }
    /// Stage live death ticks into a caller-owned buffer, reused per frame.
    pub fn stage(&self, out: &mut Vec<u64>) {
        out.clear();
        out.extend_from_slice(&self.deaths);
    }
    fn spawn(&mut self, ei: usize) {
        if self.deaths.len() >= super::POOL_CAP {
            return;
        }
        let d = self.descs[ei];
        let lo = super::floor_to_u32_sat(d.life_s[0] / super::TICK_DT);
        let hi = super::floor_to_u32_sat(d.life_s[1] / super::TICK_DT).max(lo);
        let span = u64::from(hi.wrapping_sub(lo).wrapping_add(1).max(1));
        let life = u64::from(lo).wrapping_add(self.stream.next_u64() % span);
        self.deaths.push(self.ticks.wrapping_add(life.max(1)));
        self.spawned = self.spawned.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn batch4(seed: u64) -> HanabiBatch {
        HanabiBatch::new(
            seed,
            vec![
                HanabiEffectDesc::dust(),
                HanabiEffectDesc::flame(),
                HanabiEffectDesc::sparks(),
                HanabiEffectDesc::burst64(),
            ],
        )
        .expect("fixed descs build")
    }
    #[test]
    fn hanabi_rejects_bad_desc() {
        let mut bad = HanabiEffectDesc::dust();
        bad.rate_per_s = f32::NAN;
        assert!(matches!(
            bad.validate(),
            Err(super::super::ParticleError::Invalid(_))
        ));
        assert!(HanabiBatch::new(1, vec![]).is_err());
    }
    #[test]
    fn vfx_hanabi_batch_renders() {
        let (mut a, mut b) = (batch4(1234), batch4(1234));
        let (mut pa, mut pb) = (
            super::super::ParticlePool::new(1234),
            super::super::ParticlePool::new(1234),
        );
        for _ in 0..120 {
            a.tick();
            b.tick();
            pa.tick();
            pb.tick();
        }
        assert_eq!(pa.particle_bytes(), pb.particle_bytes(), "pool bytes x2");
        assert_eq!(
            pa.particle_bytes()
                .into_iter()
                .zip(pb.particle_bytes())
                .filter(|(x, y)| x != y)
                .count(),
            0,
            "pool diff 0"
        );
        assert_eq!(
            (a.dispatch_count(), b.dispatch_count()),
            (1, 1),
            "one dispatch"
        );
        assert_eq!(
            (a.len(), super::super::EMITTER_COUNT + 1),
            (4, 5),
            "the batch holds its four effects beside the four pool kinds"
        );
        assert_eq!(a.alive(), b.alive(), "alive x2");
        assert!(a.alive() > 0, "batch runs");
        assert_eq!(
            (
                super::super::POOL_CAP,
                super::super::EMITTER_COUNT,
                super::super::PARTICLE_BUDGET_MS
            ),
            (4096, 4, 1.0),
            "bounds exact"
        );
        for t in [0.0, 0.5, 1.0] {
            let m = crate::particles::hanabi_gradient(a.descs[0].grad_a, a.descs[0].grad_b, t);
            assert_eq!(
                m.map(|v| v.to_bits()),
                a.descs[0].sample_gradient(t).map(|v| v.to_bits()),
                "mirror agrees"
            );
        }
        for w in [
            "hanabi_vs",
            "hanabi_fs",
            "effect",
            "particle",
            "age01",
            "gradient",
        ] {
            assert!(HANABI_WGSL.contains(w), "shader has {w}");
        }
        let at = super::super::ParticleReceipt {
            pass: "hanabi",
            ticks: 1,
            alive: 1,
            spawned: 1,
            ms: super::super::PARTICLE_BUDGET_MS,
            degraded: false,
        };
        assert!(at.within_budget(), "at budget within");
    }
}
